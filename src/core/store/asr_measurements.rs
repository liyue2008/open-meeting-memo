use super::*;
use crate::core::contracts::{
    AsrChunkTiming, AsrStartupTiming, AsrWorkerEvent, AudioSource, TaskContext,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Sample {
    schema_version: u32,
    task_id: Uuid,
    attempt: u32,
    lease: Uuid,
    observed_at_ms: i64,
    source: AudioSource,
    timing: AsrChunkTiming,
    startup_timing: Option<AsrStartupTiming>,
}
#[derive(Default, Serialize, Deserialize)]
struct Totals {
    observations: u64,
    cache_hits: u64,
    native_decode_wall_us: u64,
    process_cpu_us: u64,
    cpu_observations: u64,
    cache_lookup_us: u64,
    checkpoint_write_us: u64,
}
impl Store {
    pub fn record_asr_measurement_best_effort(
        &self,
        context: &TaskContext,
        event: &AsrWorkerEvent,
    ) {
        if let Err(error) = self.record_asr_measurement(context, event) {
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr().lock(),
                "warning: ASR measurement unavailable code={:?}",
                error.code
            );
        }
    }
    fn record_asr_measurement(&self, c: &TaskContext, event: &AsrWorkerEvent) -> Result<()> {
        let AsrWorkerEvent::ChunkProgress {
            schema_version,
            task_id,
            attempt,
            source,
            chunk_timing: Some(timing),
            startup_timing,
            ..
        } = event
        else {
            return Ok(());
        };
        if *schema_version != 1 || *task_id != c.task_id || *attempt != c.attempt || !timing.valid()
        {
            return Ok(());
        }
        let now = chrono::Utc::now().timestamp_millis();
        let sample = Sample {
            schema_version: 1,
            task_id: *task_id,
            attempt: *attempt,
            lease: c.lease.token,
            observed_at_ms: now,
            source: *source,
            timing: timing.clone(),
            startup_timing: startup_timing.filter(AsrStartupTiming::valid),
        };
        let document = encode(&sample)?;
        if document.len() > 1024 {
            return Ok(());
        }
        // Observer tables are lazy: old read-only databases need no migration to query diagnostics.
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS asr_measurements(task TEXT NOT NULL,slot INTEGER NOT NULL,ordinal INTEGER NOT NULL,document TEXT NOT NULL CHECK(length(CAST(document AS BLOB))<=1024),PRIMARY KEY(task,slot)); CREATE TABLE IF NOT EXISTS asr_measurement_totals(task TEXT PRIMARY KEY,document TEXT NOT NULL);")?;
        let tx = self.conn.unchecked_transaction()?;
        let live:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND attempt=?2 AND lease=?3 AND expires>?4 AND status='transcribing') OR EXISTS(SELECT 1 FROM segment_jobs s JOIN tasks t ON t.session=s.session WHERE t.id=?1 AND t.attempt=?2 AND s.lease=?3 AND s.expires>?4 AND s.status='running')",params![c.task_id.to_string(),c.attempt,c.lease.token.to_string(),now],|r|r.get(0))?;
        if !live {
            return Ok(());
        }
        let duplicate:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM asr_measurements WHERE task=?1 AND json_extract(document,'$.lease')=?2 AND json_extract(document,'$.source')=?3 AND json_extract(document,'$.timing.input_index')=?4 AND json_extract(document,'$.timing.window_index')=?5)",params![c.task_id.to_string(),c.lease.token.to_string(),source_name(*source),timing.input_index,timing.window_index],|r|r.get(0))?;
        if duplicate {
            return Ok(());
        }
        let prior: Option<String> = tx
            .query_row(
                "SELECT document FROM asr_measurement_totals WHERE task=?1",
                [c.task_id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let mut totals: Totals = prior
            .as_deref()
            .map(decode)
            .transpose()?
            .unwrap_or_default();
        totals.observations = totals.observations.saturating_add(1);
        totals.cache_hits = totals
            .cache_hits
            .saturating_add(u64::from(timing.cache_hit));
        totals.native_decode_wall_us = totals
            .native_decode_wall_us
            .saturating_add(timing.native_decode_wall_us);
        if let Some(cpu) = timing.process_cpu_us {
            totals.cpu_observations = totals.cpu_observations.saturating_add(1);
            totals.process_cpu_us = totals.process_cpu_us.saturating_add(cpu);
        }
        totals.cache_lookup_us = totals
            .cache_lookup_us
            .saturating_add(timing.cache_lookup_us);
        totals.checkpoint_write_us = totals
            .checkpoint_write_us
            .saturating_add(timing.checkpoint_write_us);
        tx.execute("INSERT INTO asr_measurements(task,slot,ordinal,document) VALUES(?1,?2,?3,?4) ON CONFLICT(task,slot) DO UPDATE SET ordinal=excluded.ordinal,document=excluded.document",params![c.task_id.to_string(),totals.observations%256,totals.observations.min(i64::MAX as u64),document])?;
        tx.execute("INSERT INTO asr_measurement_totals(task,document) VALUES(?1,?2) ON CONFLICT(task) DO UPDATE SET document=excluded.document",params![c.task_id.to_string(),encode(&totals)?])?;
        tx.commit()?;
        Ok(())
    }
    pub fn asr_measurements(&self, task: Uuid) -> Result<serde_json::Value> {
        let exists:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='asr_measurements')",[],|r|r.get(0))?;
        if !exists {
            return Ok(
                serde_json::json!({"schema_version":1,"samples":[],"totals":Totals::default()}),
            );
        }
        let mut stmt = self
            .conn
            .prepare("SELECT document FROM asr_measurements WHERE task=?1 ORDER BY ordinal")?;
        let samples = stmt
            .query_map([task.to_string()], |r| r.get::<_, String>(0))?
            .map(|r| Ok(serde_json::from_str::<Sample>(&r?)?))
            .collect::<Result<Vec<_>>>()?;
        let totals: Option<String> = self
            .conn
            .query_row(
                "SELECT document FROM asr_measurement_totals WHERE task=?1",
                [task.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(
            serde_json::json!({"schema_version":1,"samples":samples,"totals":totals.as_deref().map(decode::<Totals>).transpose()?.unwrap_or_default()}),
        )
    }
}
fn source_name(source: AudioSource) -> &'static str {
    match source {
        AudioSource::Mic => "mic",
        AudioSource::System => "system",
    }
}

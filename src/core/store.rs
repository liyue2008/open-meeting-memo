use super::{
    contracts::*,
    paths::{controlled_file, private_dir, sync_dir, write_private_new, Paths},
};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::PathBuf,
};
use uuid::Uuid;

pub fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
pub fn hash_file(path: &std::path::Path) -> Result<String> {
    let mut f = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = [0; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}
fn encode<T: serde::Serialize>(v: &T) -> Result<String> {
    Ok(serde_json::to_string(v)?)
}
fn decode<T: serde::de::DeserializeOwned>(v: &str) -> Result<T> {
    Ok(serde_json::from_str(v)?)
}
fn conflict() -> OmmError {
    OmmError::new(ErrorCode::Conflict, false, "task lease is stale or expired")
}

mod transaction;
use transaction::WriteConnection;
mod archives;
mod asr_measurements;
mod attempt_errors;
pub mod desktop;
mod desktop_partial;
mod notification_events;
pub use attempt_errors::{StageAttemptError, StageErrorPhase};
mod segments;
pub use segments::{ClaimedSegment, SegmentProgress};

pub struct Store {
    conn: Connection,
    paths: Paths,
}
impl Store {
    pub fn open_read_only(paths: &Paths) -> Result<Self> {
        let db = paths.state.join("state.sqlite");
        super::paths::validate_private_file(&db)?;
        let conn = Connection::open_with_flags(
            &db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.execute_batch("PRAGMA query_only=ON;")?;
        Ok(Self {
            conn,
            paths: paths.clone(),
        })
    }
    pub fn open(paths: &Paths) -> Result<Self> {
        private_dir(&paths.state)?;
        let db = paths.state.join("state.sqlite");
        if !db.exists() {
            fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&db)?;
        }
        let m = fs::symlink_metadata(&db)?;
        if !m.is_file() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
            return Err(OmmError::new(
                ErrorCode::Permission,
                false,
                "state database permissions are unsafe",
            ));
        }
        let conn = Connection::open(db)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, document TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS retries(task TEXT NOT NULL,stage TEXT NOT NULL,count INTEGER NOT NULL,next_at INTEGER NOT NULL,PRIMARY KEY(task,stage));
            CREATE TABLE IF NOT EXISTS meetings(id TEXT PRIMARY KEY, document TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS tasks(id TEXT PRIMARY KEY, session TEXT NOT NULL REFERENCES meetings(id), attempt INTEGER NOT NULL,
              input_hash TEXT NOT NULL, config_hash TEXT NOT NULL, document TEXT NOT NULL, stage TEXT NOT NULL, status TEXT NOT NULL,
              lease TEXT, expires INTEGER, directory TEXT NOT NULL, config TEXT NOT NULL, segments TEXT NOT NULL,
              UNIQUE(session, attempt));
            CREATE TABLE IF NOT EXISTS commits(task TEXT NOT NULL REFERENCES tasks(id), stage TEXT NOT NULL, lease TEXT NOT NULL,
              staging TEXT NOT NULL, destination TEXT NOT NULL, hash TEXT NOT NULL, metadata TEXT, done INTEGER NOT NULL DEFAULT 0,
              PRIMARY KEY(task,stage));")?;
        let store = Self {
            conn,
            paths: paths.clone(),
        };
        store.initialize_segments()?;
        store.initialize_archives()?;
        store.initialize_desktop()?;
        Ok(store)
    }
    pub fn state<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let doc: Option<String> = self
            .conn
            .query_row("SELECT document FROM settings WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        doc.as_deref().map(decode).transpose()
    }
    pub fn set_state<T: serde::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.conn.execute("INSERT INTO settings(key,document) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET document=excluded.document",params![key,encode(value)?])?;
        Ok(())
    }
    pub fn clear_state(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM settings WHERE key=?1", [key])?;
        Ok(())
    }
    fn audio_lock_file(paths: &Paths) -> Result<fs::File> {
        let path = paths.state.join("audio-readers.lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        super::paths::validate_private_file(&path)?;
        Ok(file)
    }
    pub fn audio_read_lock(&self) -> Result<fs::File> {
        let file = Self::audio_lock_file(&self.paths)?;
        match fs2::FileExt::try_lock_shared(&file) {
            Ok(()) => Ok(file),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Err(OmmError::new(
                ErrorCode::Conflict,
                true,
                "audio cleanup in progress",
            )),
            Err(e) => Err(e.into()),
        }
    }
    /// No blocking flock or detached waiter: dropping this future releases its file immediately.
    pub async fn audio_read_lock_until(
        paths: &Paths,
        deadline: tokio::time::Instant,
    ) -> Result<fs::File> {
        let file = Self::audio_lock_file(paths)?;
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(OmmError::new(
                    ErrorCode::Timeout,
                    true,
                    "ASR deadline expired waiting for audio maintenance",
                ));
            }
            match fs2::FileExt::try_lock_shared(&file) {
                Ok(()) => return Ok(file),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.into()),
            }
            tokio::time::sleep_until(
                deadline.min(tokio::time::Instant::now() + std::time::Duration::from_millis(25)),
            )
            .await;
        }
    }
    pub fn active_audio_readers(&self) -> Result<bool> {
        let path = self.paths.state.join("audio-readers.lock");
        if !path.exists() {
            return Ok(false);
        }
        controlled_file(&self.paths.state, &path)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(true),
            Err(e) => Err(e.into()),
        }
    }
    pub fn model_read_lock(&self) -> Result<fs::File> {
        let path = self.paths.state.join("models.lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(&path)?;
        super::paths::validate_private_file(&path)?;
        fs2::FileExt::try_lock_shared(&file).map_err(|_| {
            OmmError::new(
                ErrorCode::Conflict,
                true,
                "model installation/removal is in progress",
            )
        })?;
        Ok(file)
    }
    pub fn schedule_retry(&mut self, task: &Task, now_ms: i64) -> Result<bool> {
        if task.stage == Stage::Transcribe
            && self.is_segmented_parent(task.id)?
            && self
                .segment_progress(task.session_id)?
                .iter()
                .any(|s| s.status != "done")
        {
            return Ok(false);
        }
        if task.status != TaskStatus::Failed || !task.error.as_ref().is_some_and(|e| e.retryable) {
            return Ok(false);
        }
        let tx = self.conn.write_transaction()?;
        let count: u32 = tx
            .query_row(
                "SELECT count FROM retries WHERE task=?1 AND stage=?2",
                params![task.id.to_string(), stage_name(task.stage)],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if count >= 3 {
            return Ok(false);
        }
        let doc: String = tx.query_row(
            "SELECT document FROM tasks WHERE id=?1",
            [task.id.to_string()],
            |r| r.get(0),
        )?;
        let mut next: Task = decode(&doc)?;
        next.status = if next.stage == Stage::Summary {
            TaskStatus::TranscriptCommitted
        } else {
            TaskStatus::Queued
        };
        update_task(&tx, &next)?;
        tx.execute("INSERT INTO retries VALUES(?1,?2,?3,?4) ON CONFLICT(task,stage) DO UPDATE SET count=excluded.count,next_at=excluded.next_at",params![next.id.to_string(),stage_name(next.stage),count+1,now_ms+1000*(1i64<<count)])?;
        tx.commit()?;
        Ok(true)
    }
    pub fn enqueue(
        &mut self,
        meeting: &SealedMeeting,
        config: &ConfigSnapshot,
        force: bool,
    ) -> Result<Task> {
        // The persistent lifecycle, not an epoch timestamp, identifies a live
        // segmented meeting. Older sealed fixtures/documents legitimately use 0.
        if self.has_segmented_session(meeting.session_id)?
            && self.conn.query_row(
                "SELECT sealed=0 FROM segmented_sessions WHERE session=?1",
                [meeting.session_id.to_string()],
                |r| r.get::<_, bool>(0),
            )?
        {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "seal recording before whole-meeting processing",
            ));
        }
        if meeting.segments.is_empty() || config.version.is_empty() {
            return Err(OmmError::invalid(
                "sealed audio and configuration version are required",
            ));
        }
        let dir = meeting.directory.canonicalize()?;
        if dir != meeting.directory
            || !dir.starts_with(self.paths.data.join("meetings").canonicalize()?)
        {
            return Err(OmmError::new(
                ErrorCode::Permission,
                false,
                "meeting directory outside DATA meetings root",
            ));
        }
        for segment in &meeting.segments {
            if !segment.path.exists() && self.has_segmented_session(meeting.session_id)? {
                let receipt = self
                    .archive_for_audio(meeting.session_id, segment)?
                    .ok_or_else(|| OmmError::unavailable("source audio and archive unavailable"))?;
                for file in receipt.files {
                    controlled_file(&dir, &file.path)?;
                    if hash_file(&file.path)? != file.sha256 {
                        return Err(OmmError::new(
                            ErrorCode::Corrupt,
                            false,
                            "archive changed since commit",
                        ));
                    }
                }
                continue;
            }
            controlled_file(&dir, &segment.path)?;
            if segment.duration_ms == 0 || segment.sha256 != hash_file(&segment.path)? {
                return Err(OmmError::new(
                    ErrorCode::Corrupt,
                    false,
                    "sealed audio hash or duration is invalid",
                ));
            }
        }
        let input_hash = hash_bytes(encode(&meeting.segments)?.as_bytes());
        let config_json = encode(config)?;
        let config_hash = hash_bytes(config_json.as_bytes());
        let tx = self.conn.write_transaction()?;
        if !force {
            let existing: Option<String> = tx.query_row("SELECT document FROM tasks WHERE session=?1 AND input_hash=?2 AND config_hash=?3 ORDER BY attempt DESC LIMIT 1", params![meeting.session_id.to_string(), input_hash, config_hash], |r| r.get(0)).optional()?;
            if let Some(document) = existing {
                return decode(&document);
            }
        }
        let existing_meeting: Option<String> = tx
            .query_row(
                "SELECT document FROM meetings WHERE id=?1",
                [meeting.session_id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(document) = existing_meeting {
            if document != encode(meeting)? {
                return Err(OmmError::new(
                    ErrorCode::Conflict,
                    false,
                    "sealed meeting metadata is immutable",
                ));
            }
        } else {
            tx.execute(
                "INSERT INTO meetings VALUES(?1,?2)",
                params![meeting.session_id.to_string(), encode(meeting)?],
            )?;
        }
        let attempt: u32 = tx.query_row(
            "SELECT COALESCE(MAX(attempt),0)+1 FROM tasks WHERE session=?1",
            [meeting.session_id.to_string()],
            |r| r.get(0),
        )?;
        let task = Task {
            id: Uuid::new_v4(),
            session_id: meeting.session_id,
            attempt,
            stage: Stage::Transcribe,
            status: TaskStatus::Queued,
            input_hash,
            config_hash,
            summary_metadata: None,
            transcript_source: None,
            error: None,
        };
        tx.execute("INSERT INTO tasks(id,session,attempt,input_hash,config_hash,document,stage,status,directory,config,segments) VALUES(?1,?2,?3,?4,?5,?6,'transcribe','queued',?7,?8,?9)", params![task.id.to_string(), task.session_id.to_string(), task.attempt, task.input_hash, task.config_hash, encode(&task)?, dir.to_str(), config_json, encode(&meeting.segments)?])?;
        tx.commit()?;
        Ok(task)
    }
    /// Copy an already committed transcript under a fresh import lease. The ASR
    /// snapshot remains the source snapshot; only Pi/prompt configuration changes.
    pub fn force_summary(&mut self, source_id: Uuid, version: &str, pi: &PiConfig) -> Result<Task> {
        let source = self.task(source_id)?;
        let bytes = self.read_artifact(source_id, Stage::Transcribe)?;
        let transcript: Transcript = serde_json::from_slice(&bytes)?;
        let (dir, config_json, segments_json): (String, String, String) = self.conn.query_row(
            "SELECT directory,config,segments FROM tasks WHERE id=?1",
            [source_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let mut config: ConfigSnapshot = decode(&config_json)?;
        let segments: Vec<AudioSegment> = decode(&segments_json)?;
        let duration = self.transcript_duration(&source, &segments)?;
        if source.id != source_id
            || hash_bytes(config_json.as_bytes()) != source.config_hash
            || hash_bytes(segments_json.as_bytes()) != source.input_hash
            || transcript.schema_version != 1
            || transcript.model_version != config.asr.model_version
            || transcript.duration_ms != duration
            || transcript
                .segments
                .iter()
                .any(|s| s.start_ms >= s.end_ms || s.end_ms > duration || s.text.trim().is_empty())
        {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "source transcript provenance is invalid",
            ));
        }
        config.version = version.to_owned();
        config.pi = pi.clone();
        let new_config_json = encode(&config)?;
        let tx = self.conn.write_transaction()?;
        let attempt: u32 = tx.query_row(
            "SELECT COALESCE(MAX(attempt),0)+1 FROM tasks WHERE session=?1",
            [source.session_id.to_string()],
            |r| r.get(0),
        )?;
        let task = Task {
            id: Uuid::new_v4(),
            session_id: source.session_id,
            attempt,
            stage: Stage::Transcribe,
            status: TaskStatus::Transcribing,
            input_hash: source.input_hash.clone(),
            config_hash: hash_bytes(new_config_json.as_bytes()),
            summary_metadata: None,
            transcript_source: Some(TranscriptSource {
                task_id: source_id,
                sha256: hash_bytes(&bytes),
                model_version: transcript.model_version,
                config_hash: source.config_hash,
            }),
            error: None,
        };
        let lease = Uuid::new_v4();
        let parent = PathBuf::from(&dir).join(format!("attempt-{:04}-{}", attempt, task.id));
        private_dir(&parent)?;
        private_dir(&parent.join("staging"))?;
        let staging = parent.join("staging").join(format!("{}.tmp", lease));
        let destination = parent.join("transcript.json");
        write_private_new(&staging, &bytes)?;
        sync_dir(staging.parent().unwrap())?;
        tx.execute("INSERT INTO tasks(id,session,attempt,input_hash,config_hash,document,stage,status,lease,expires,directory,config,segments) VALUES(?1,?2,?3,?4,?5,?6,'transcribe','transcribing',?7,?8,?9,?10,?11)",params![task.id.to_string(),task.session_id.to_string(),attempt,task.input_hash,task.config_hash,encode(&task)?,lease.to_string(),chrono::Utc::now().timestamp_millis()+60000,dir,new_config_json,segments_json])?;
        tx.execute("INSERT INTO commits(task,stage,lease,staging,destination,hash) VALUES(?1,'transcribe',?2,?3,?4,?5)",params![task.id.to_string(),lease.to_string(),staging.to_str(),destination.to_str(),hash_bytes(&bytes)])?;
        tx.commit()?;
        self.recover_commits()?;
        self.task(task.id)
    }
    pub fn task(&self, id: Uuid) -> Result<Task> {
        let doc: Option<String> = self
            .conn
            .query_row(
                "SELECT document FROM tasks WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        decode(&doc.ok_or_else(|| OmmError::invalid("task not found"))?)
    }
    pub fn tasks(&self) -> Result<Vec<Task>> {
        let mut stmt = self
            .conn
            .prepare("SELECT document FROM tasks ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|row| decode(&row?)).collect()
    }
    pub fn meetings(&self) -> Result<Vec<SealedMeeting>> {
        let mut stmt = self
            .conn
            .prepare("SELECT document FROM meetings ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|row| decode(&row?)).collect()
    }
    pub fn claim(&mut self, now_ms: i64, lease_ms: i64) -> Result<Option<ClaimedTask>> {
        self.claim_lane(now_ms, lease_ms, None, false)
    }
    pub fn claim_asr(&mut self, now_ms: i64, lease_ms: i64) -> Result<Option<ClaimedTask>> {
        self.claim_lane(now_ms, lease_ms, Some(Stage::Transcribe), false)
    }
    pub fn claim_summary(&mut self, now_ms: i64, lease_ms: i64) -> Result<Option<ClaimedTask>> {
        self.claim_lane(now_ms, lease_ms, Some(Stage::Summary), false)
    }
    /// Only sealed, fully checkpointed parents: this path never owns an audio decoder.
    pub fn claim_ready_aggregate(
        &mut self,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<Option<ClaimedTask>> {
        self.claim_lane(now_ms, lease_ms, Some(Stage::Transcribe), true)
    }
    fn claim_lane(
        &mut self,
        now_ms: i64,
        lease_ms: i64,
        lane: Option<Stage>,
        aggregate_only: bool,
    ) -> Result<Option<ClaimedTask>> {
        let filter = lane.map(stage_name);
        if lease_ms <= 0 {
            return Err(OmmError::invalid("lease duration must be positive"));
        }
        self.recover(now_ms)?;
        let tx = self.conn.write_transaction()?;
        if desktop::service_stopping(&tx)? {
            return Ok(None);
        }
        if !aggregate_only && lane!=Some(Stage::Summary) && super::resources::CpuBudget::current().shared_encoder_slot && tx.query_row("SELECT EXISTS(SELECT 1 FROM segment_archives WHERE lease IS NOT NULL AND expires>?1)",[now_ms],|r|r.get::<_,bool>(0))?{return Ok(None);}
        let running: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE lease IS NOT NULL AND expires>?1 AND (?2 IS NULL OR stage=?2))",
            params![now_ms,filter],
            |r| r.get(0),
        )?;
        let segment_running: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM segment_jobs WHERE lease IS NOT NULL AND expires>?1)",
            [now_ms],
            |r| r.get(0),
        )?;
        if !aggregate_only && (running || (lane != Some(Stage::Summary) && segment_running)) {
            return Ok(None);
        }
        let row: Option<(String,String,String,String)> = tx.query_row("SELECT document,directory,config,segments FROM tasks WHERE NOT EXISTS(SELECT 1 FROM desktop_cancellations c WHERE c.task=tasks.id) AND status IN ('queued','transcript_committed','interrupted') AND (?2 IS NULL OR tasks.stage=?2) AND (?3=0 OR EXISTS(SELECT 1 FROM segmented_sessions s WHERE s.session=tasks.session AND s.parent=tasks.id)) AND NOT EXISTS(SELECT 1 FROM segmented_sessions s WHERE s.session=tasks.session AND s.parent=tasks.id AND (s.sealed=0 OR NOT EXISTS(SELECT 1 FROM segment_jobs j WHERE j.session=s.session AND j.status='done' AND j.frames>0) OR EXISTS(SELECT 1 FROM segment_jobs j WHERE j.session=s.session AND j.status!='done'))) AND NOT EXISTS(SELECT 1 FROM retries WHERE retries.task=tasks.id AND retries.stage=tasks.stage AND retries.next_at>?1) ORDER BY rowid LIMIT 1", params![now_ms,filter,aggregate_only], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let Some((doc, dir, config, segments)) = row else {
            return Ok(None);
        };
        let mut task: Task = decode(&doc)?;
        task.status = if task.stage == Stage::Transcribe {
            TaskStatus::Transcribing
        } else {
            TaskStatus::Summarizing
        };
        task.error = None;
        let lease = Lease {
            token: Uuid::new_v4(),
            expires_at_ms: now_ms
                .checked_add(lease_ms)
                .ok_or_else(|| OmmError::invalid("lease time overflow"))?,
        };
        tx.execute(
            "UPDATE tasks SET document=?1,status=?2,lease=?3,expires=?4 WHERE id=?5",
            params![
                encode(&task)?,
                status_name(task.status),
                lease.token.to_string(),
                lease.expires_at_ms,
                task.id.to_string()
            ],
        )?;
        tx.commit()?;
        let meeting_dir = PathBuf::from(dir);
        let work_dir = self
            .paths
            .cache
            .join("tasks")
            .join(task.id.to_string())
            .join(lease.token.to_string());
        private_dir(&work_dir)?;
        let context = TaskContext {
            session_id: task.session_id,
            task_id: task.id,
            attempt: task.attempt,
            lease: lease.clone(),
            meeting_dir,
            work_dir,
            config: decode(&config)?,
        };
        Ok(Some(ClaimedTask {
            stage: task.stage,
            task,
            lease,
            context,
            segments: decode(&segments)?,
        }))
    }
    pub fn renew(&mut self, claim: &mut ClaimedTask, now_ms: i64, lease_ms: i64) -> Result<()> {
        let tx = self.conn.write_transaction()?;
        check_lease(&tx, claim, now_ms)?;
        let expires = now_ms
            .checked_add(lease_ms)
            .filter(|_| lease_ms > 0)
            .ok_or_else(|| OmmError::invalid("invalid renewal duration"))?;
        tx.execute(
            "UPDATE tasks SET expires=?1 WHERE id=?2",
            params![expires, claim.task.id.to_string()],
        )?;
        tx.commit()?;
        claim.lease.expires_at_ms = expires;
        claim.context.lease.expires_at_ms = expires;
        Ok(())
    }
    pub fn commit_transcript(
        &mut self,
        claim: &ClaimedTask,
        transcript: &Transcript,
        now_ms: i64,
    ) -> Result<()> {
        if claim.stage != Stage::Transcribe {
            return Err(conflict());
        }
        let duration = self.transcript_duration(&claim.task, &claim.segments)?;
        if transcript.schema_version != 1
            || transcript.model_version.is_empty()
            || transcript.model_version != claim.context.config.asr.model_version
            || transcript.duration_ms != duration
            || transcript
                .segments
                .iter()
                .any(|s| s.start_ms >= s.end_ms || s.end_ms > duration || s.text.trim().is_empty())
        {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "transcript schema or timestamps are invalid",
            ));
        }
        self.commit(claim, serde_json::to_vec_pretty(transcript)?, None, now_ms)
    }
    pub fn commit_summary(
        &mut self,
        claim: &ClaimedTask,
        summary: &Summary,
        now_ms: i64,
    ) -> Result<()> {
        if claim.stage != Stage::Summary {
            return Err(conflict());
        }
        if summary.schema_version != 1
            || summary.markdown.trim().is_empty()
            || summary.provider.is_empty()
            || summary.model.is_empty()
            || summary.prompt_version.is_empty()
            || summary.provider != claim.context.config.pi.provider
            || summary.model != claim.context.config.pi.model
            || summary.prompt_version != claim.context.config.pi.prompt_version
        {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "summary metadata or body is invalid",
            ));
        }
        let metadata = SummaryMetadata {
            provider: summary.provider.clone(),
            model: summary.model.clone(),
            prompt_version: summary.prompt_version.clone(),
        };
        self.commit(
            claim,
            summary.markdown.as_bytes().to_vec(),
            Some(metadata),
            now_ms,
        )
    }
    fn commit(
        &mut self,
        claim: &ClaimedTask,
        bytes: Vec<u8>,
        metadata: Option<SummaryMetadata>,
        now_ms: i64,
    ) -> Result<()> {
        let destination = self.artifact_path(claim.task.id, claim.stage)?;
        let parent = destination
            .parent()
            .ok_or_else(|| OmmError::invalid("artifact parent missing"))?;
        private_dir(parent)?;
        private_dir(&parent.join("staging"))?;
        let staging = parent
            .join("staging")
            .join(format!("{}.tmp", claim.lease.token));
        let tx = self.conn.write_transaction()?;
        check_lease(&tx, claim, now_ms)?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM desktop_cancellations WHERE task=?1)",
            [claim.task.id.to_string()],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(OmmError::new(
                ErrorCode::Unavailable,
                false,
                "task cancellation requested",
            ));
        }
        write_private_new(&staging, &bytes)?;
        sync_dir(staging.parent().unwrap())?;
        tx.execute("INSERT INTO commits(task,stage,lease,staging,destination,hash,metadata) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![claim.task.id.to_string(), stage_name(claim.stage), claim.lease.token.to_string(), staging.to_str(), destination.to_str(), hash_bytes(&bytes), metadata.as_ref().map(encode).transpose()?])?;
        tx.commit()?;
        self.recover_commits()?;
        Ok(())
    }
    pub fn artifact_path(&self, id: Uuid, stage: Stage) -> Result<PathBuf> {
        let dir: String = self.conn.query_row(
            "SELECT directory FROM tasks WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?;
        Ok(PathBuf::from(dir)
            .join(format!("attempt-{:04}-{}", self.task(id)?.attempt, id))
            .join(if stage == Stage::Transcribe {
                "transcript.json"
            } else {
                "summary.md"
            }))
    }
    pub fn artifact_committed(&self, id: Uuid, stage: Stage) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM commits WHERE task=?1 AND stage=?2 AND done=1)",
            params![id.to_string(), stage_name(stage)],
            |r| r.get(0),
        )?)
    }
    pub fn read_artifact(&self, id: Uuid, stage: Stage) -> Result<Vec<u8>> {
        let hash: Option<String> = self
            .conn
            .query_row(
                "SELECT hash FROM commits WHERE task=?1 AND stage=?2 AND done=1",
                params![id.to_string(), stage_name(stage)],
                |r| r.get(0),
            )
            .optional()?;
        let expected = hash.ok_or_else(|| OmmError::unavailable("artifact not committed"))?;
        let path = self.artifact_path(id, stage)?;
        controlled_file(&self.paths.data, &path)?;
        let bytes = fs::read(path)?;
        if hash_bytes(&bytes) != expected {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "committed artifact hash mismatch",
            ));
        }
        Ok(bytes)
    }
    pub fn fail(&mut self, claim: &ClaimedTask, error: &OmmError, now_ms: i64) -> Result<()> {
        let tx = self.conn.write_transaction()?;
        check_lease(&tx, claim, now_ms)?;
        let pending: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM commits WHERE task=?1 AND done=0)",
            [claim.task.id.to_string()],
            |r| r.get(0),
        )?;
        if pending {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                true,
                "artifact commit pending recovery; lease retained",
            ));
        }
        let mut task = claim.task.clone();
        task.status = if matches!(error.code, ErrorCode::Unavailable | ErrorCode::Permission) {
            TaskStatus::Blocked
        } else {
            TaskStatus::Failed
        };
        task.error = Some(error.clone());
        update_task(&tx, &task)?;
        tx.commit()?;
        Ok(())
    }
    pub fn retry(&mut self, id: Uuid) -> Result<()> {
        let tx = self.conn.write_transaction()?;
        let doc: String = tx.query_row(
            "SELECT document FROM tasks WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?;
        let mut task: Task = decode(&doc)?;
        if !matches!(
            task.status,
            TaskStatus::Failed | TaskStatus::Blocked | TaskStatus::Interrupted
        ) {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "only failed, blocked or interrupted tasks can retry",
            ));
        }
        task.status = if task.stage == Stage::Summary {
            TaskStatus::TranscriptCommitted
        } else {
            TaskStatus::Queued
        };
        task.error = None;
        update_task(&tx, &task)?;
        tx.execute("DELETE FROM retries WHERE task=?1", [id.to_string()])?;
        tx.commit()?;
        Ok(())
    }
    pub fn clean_abandoned_cache(&self, now_ms: i64) -> Result<()> {
        if self.active_audio_readers()? {
            return Ok(());
        }
        let cache = self.paths.cache.join("tasks");
        if !cache.exists() {
            return Ok(());
        }
        super::paths::validate_private_dir(&cache)?;
        for entry in fs::read_dir(&cache)? {
            let entry = entry?;
            let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|s| Uuid::parse_str(s).ok())
            else {
                continue;
            };
            let status: Option<String> = self
                .conn
                .query_row(
                    "SELECT status FROM tasks WHERE id=?1",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            // Unknown ownership is retained, not inferred from a UUID-shaped directory.
            let Some(status) = status else { continue };
            let active: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND lease IS NOT NULL AND expires>?2) OR EXISTS(SELECT 1 FROM segment_jobs JOIN tasks ON tasks.session=segment_jobs.session WHERE tasks.id=?1 AND segment_jobs.status='running' AND segment_jobs.expires>?2)",params![id.to_string(),now_ms],|r|r.get(0))?;
            if active {
                continue;
            }
            super::paths::validate_private_dir(&entry.path())?;
            for child in fs::read_dir(entry.path())? {
                let child = child?;
                let name = child.file_name();
                let chunks = name == ".chunks";
                // .chunks is durable across queued/failed/blocked/restart. Only lease
                // UUID directories are disposable; arbitrary entries are never erased.
                if (chunks && status == "completed")
                    || name
                        .to_str()
                        .and_then(|s| Uuid::parse_str(s).ok())
                        .is_some()
                {
                    super::paths::validate_private_dir(&child.path())?;
                    fs::remove_dir_all(child.path())?;
                }
            }
            if fs::read_dir(entry.path())?.next().is_none() {
                fs::remove_dir(entry.path())?;
            }
        }
        Ok(())
    }
    pub fn recover(&mut self, now_ms: i64) -> Result<()> {
        self.recover_commits()?;
        let tx = self.conn.write_transaction()?;
        let docs = {
            let mut stmt =
                tx.prepare("SELECT document FROM tasks WHERE lease IS NOT NULL AND expires<=?1")?;
            let rows = stmt.query_map([now_ms], |r| r.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for doc in docs {
            let mut task: Task = decode(&doc)?;
            task.status = TaskStatus::Interrupted;
            update_task(&tx, &task)?;
        }
        tx.commit()?;
        Ok(())
    }
    fn recover_commits(&mut self) -> Result<()> {
        let rows = {
            let mut stmt = self.conn.prepare("SELECT task,stage,lease,staging,destination,hash,metadata FROM commits WHERE done=0")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<String>>(6)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (id, stage, token, tmp, dest, hash, metadata) in rows {
            let tx = self.conn.write_transaction()?;
            let done: bool = tx.query_row(
                "SELECT done FROM commits WHERE task=?1 AND stage=?2",
                params![id, stage],
                |r| r.get(0),
            )?;
            if done {
                continue;
            }
            let (doc, current): (String, Option<String>) =
                tx.query_row("SELECT document,lease FROM tasks WHERE id=?1", [&id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
            let mut task: Task = decode(&doc)?;
            if current.as_deref() != Some(&token) || stage_name(task.stage) != stage {
                return Err(conflict());
            }
            let destination = PathBuf::from(dest);
            let staging = PathBuf::from(tmp);
            if destination.exists() {
                controlled_file(&self.paths.data, &destination)?;
                if hash_file(&destination)? != hash {
                    return Err(OmmError::new(
                        ErrorCode::Corrupt,
                        false,
                        "pending artifact differs from journal",
                    ));
                }
            } else {
                controlled_file(&self.paths.data, &staging)?;
                if hash_file(&staging)? != hash {
                    return Err(OmmError::new(
                        ErrorCode::Corrupt,
                        false,
                        "staging hash mismatch",
                    ));
                }
                fs::rename(&staging, &destination)?;
                sync_dir(destination.parent().unwrap())?;
            }
            if task.stage == Stage::Transcribe {
                task.stage = Stage::Summary;
                task.status = TaskStatus::TranscriptCommitted;
            } else {
                task.status = TaskStatus::Completed;
                task.summary_metadata = metadata.as_deref().map(decode).transpose()?;
            }
            task.error = None;
            update_task(&tx, &task)?;
            tx.execute(
                "UPDATE commits SET done=1 WHERE task=?1 AND stage=?2",
                params![id, stage],
            )?;
            tx.commit()?;
            if staging.exists() {
                fs::remove_file(staging)?;
            }
        }
        self.refresh_desktop_search()?;
        Ok(())
    }
}
fn check_lease(tx: &Connection, claim: &ClaimedTask, now_ms: i64) -> Result<()> {
    let (token, expires, stage): (Option<String>, Option<i64>, String) = tx.query_row(
        "SELECT lease,expires,stage FROM tasks WHERE id=?1",
        [claim.task.id.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if token.as_deref() != Some(claim.lease.token.to_string().as_str())
        || expires.unwrap_or(0) <= now_ms
        || stage != stage_name(claim.stage)
    {
        return Err(conflict());
    }
    Ok(())
}
fn update_task(tx: &Connection, task: &Task) -> Result<()> {
    tx.execute(
        "UPDATE tasks SET document=?1,stage=?2,status=?3,lease=NULL,expires=NULL WHERE id=?4",
        params![
            encode(task)?,
            stage_name(task.stage),
            status_name(task.status),
            task.id.to_string()
        ],
    )?;
    Ok(())
}
fn stage_name(stage: Stage) -> &'static str {
    match stage {
        Stage::Transcribe => "transcribe",
        Stage::Summary => "summary",
    }
}
fn status_name(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Queued => "queued",
        TaskStatus::Transcribing => "transcribing",
        TaskStatus::TranscriptCommitted => "transcript_committed",
        TaskStatus::Summarizing => "summarizing",
        TaskStatus::Completed => "completed",
        TaskStatus::Blocked => "blocked",
        TaskStatus::Failed => "failed",
        TaskStatus::Interrupted => "interrupted",
    }
}

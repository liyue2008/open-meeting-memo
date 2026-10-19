//! Additive desktop metadata. Sealed capture manifests and task attempts remain immutable.
use super::*;
use crate::application::desktop_contracts as dto;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(super) fn service_stopping(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM settings WHERE key='desktop_service_stopping')",
        [],
        |r| r.get(0),
    )?)
}
fn processing_busy(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE lease IS NOT NULL OR status IN ('transcribing','summarizing')) OR EXISTS(SELECT 1 FROM segment_jobs WHERE lease IS NOT NULL OR status='running') OR EXISTS(SELECT 1 FROM segment_archives WHERE lease IS NOT NULL)",[],|r|r.get(0))?)
}
fn missing() -> OmmError {
    OmmError::new(ErrorCode::Unavailable, false, "meeting not found")
}
fn changed() -> OmmError {
    OmmError::new(
        ErrorCode::Conflict,
        false,
        "meeting revision changed; reload before saving",
    )
}
impl Store {
    pub(super) fn initialize_desktop(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS desktop_metadata(meeting TEXT PRIMARY KEY REFERENCES meetings(id),revision INTEGER NOT NULL,document TEXT NOT NULL,trashed INTEGER NOT NULL DEFAULT 0,started_at INTEGER,time_source TEXT NOT NULL DEFAULT 'estimated',source TEXT NOT NULL DEFAULT 'legacy');
        CREATE TABLE IF NOT EXISTS desktop_requests(request TEXT PRIMARY KEY,body_hash TEXT NOT NULL,result TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS desktop_cancellations(task TEXT PRIMARY KEY REFERENCES tasks(id),requested_at INTEGER NOT NULL,settled INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS desktop_imports(hash TEXT PRIMARY KEY,meeting TEXT NOT NULL); CREATE TABLE IF NOT EXISTS desktop_search(task TEXT NOT NULL,stage TEXT NOT NULL,hash TEXT NOT NULL,content TEXT NOT NULL,PRIMARY KEY(task,stage));")?;
        Ok(())
    }
    pub fn desktop_current_artifact(&self, id: Uuid, stage: Stage) -> Result<Option<Uuid>> {
        let value:Option<String>=self.conn.query_row("SELECT c.task FROM commits c JOIN tasks t ON t.id=c.task WHERE t.session=?1 AND c.stage=?2 AND c.done=1 ORDER BY t.attempt DESC,t.rowid DESC LIMIT 1",params![id.to_string(),stage_name(stage)],|r|r.get(0)).optional()?;
        value
            .map(|id| Uuid::parse_str(&id).map_err(|_| missing()))
            .transpose()
    }
    pub(super) fn refresh_desktop_search(&self) -> Result<()> {
        let rows = {
            let mut stmt=self.conn.prepare("SELECT c.task,c.stage,c.hash FROM commits c LEFT JOIN desktop_search s ON s.task=c.task AND s.stage=c.stage WHERE c.done=1 AND (s.hash IS NULL OR s.hash!=c.hash)")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (task, stage, hash) in rows {
            let id = Uuid::parse_str(&task).map_err(|_| missing())?;
            let kind = if stage == "transcribe" {
                Stage::Transcribe
            } else {
                Stage::Summary
            };
            let bytes = self.read_artifact(id, kind)?;
            let text = if kind == Stage::Transcribe {
                serde_json::from_slice::<Transcript>(&bytes)?
                    .segments
                    .into_iter()
                    .map(|s| s.text)
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                String::from_utf8(bytes).map_err(|_| OmmError::invalid("summary must be UTF-8"))?
            };
            self.conn.execute(
                "INSERT OR REPLACE INTO desktop_search VALUES(?1,?2,?3,?4)",
                params![task, stage, hash, text],
            )?;
        }
        Ok(())
    }
    pub fn has_desktop_schema(&self) -> Result<bool> {
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='desktop_metadata')",[],|r|r.get(0))?)
    }
    pub fn desktop_atomic(
        &mut self,
        command: &str,
        request: &Value,
        operation: impl FnOnce(&mut Self) -> Result<Value>,
    ) -> Result<Value> {
        let key = format!(
            "{}:{}",
            command,
            request["requestId"]
                .as_str()
                .ok_or_else(|| OmmError::invalid("missing request identifier"))?
        );
        let mut canonical = request.clone();
        canonical["payload"]
            .as_object_mut()
            .ok_or_else(|| OmmError::invalid("missing payload"))?
            .remove("admission");
        let hash = hash_bytes(&serde_json::to_vec(&canonical)?);
        if !self.conn.is_autocommit() {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "desktop operation cannot nest another desktop operation",
            ));
        }
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(result) = self.desktop_request(&key, &hash)? {
                return Ok(result);
            }
            let result = operation(self)?;
            self.desktop_record_request(&key, &hash, &result)?;
            Ok::<_, OmmError>(result)
        }));
        match result {
            Ok(Ok(value)) => {
                if let Err(error) = self.conn.execute_batch("COMMIT") {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    return Err(error.into());
                }
                Ok(value)
            }
            Ok(Err(error)) => {
                self.conn.execute_batch("ROLLBACK")?;
                Err(error)
            }
            Err(panic) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                std::panic::resume_unwind(panic)
            }
        }
    }
    pub fn desktop_request(&self, key: &str, hash: &str) -> Result<Option<Value>> {
        let previous: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT body_hash,result FROM desktop_requests WHERE request=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        previous
            .map(|(body, result)| {
                if body == hash {
                    decode(&result)
                } else {
                    Err(OmmError::new(
                        ErrorCode::Conflict,
                        false,
                        "request identifier reused with different payload",
                    ))
                }
            })
            .transpose()
    }
    pub fn desktop_record_request(&self, key: &str, hash: &str, result: &Value) -> Result<()> {
        self.conn.execute(
            "INSERT INTO desktop_requests VALUES(?1,?2,?3)",
            params![key, hash, encode(result)?],
        )?;
        Ok(())
    }
    pub fn desktop_row(&self, id: Uuid) -> Result<Value> {
        let meeting = self.meeting(id)?.ok_or_else(missing)?;
        let tasks = self
            .tasks()?
            .into_iter()
            .filter(|t| t.session_id == id)
            .collect::<Vec<_>>();
        let (metadata, revision, trashed, started, time_source, source): (
            Value,
            i64,
            bool,
            Option<i64>,
            String,
            String,
        ) = if self.has_desktop_schema()? {
            self.conn.query_row("SELECT document,revision,trashed,started_at,time_source,source FROM desktop_metadata WHERE meeting=?1",[id.to_string()],|r|Ok((r.get::<_,String>(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?.map(|(document,revision,trashed,started,time_source,source)|Ok::<_,OmmError>((decode(&document)?,revision,trashed,started,time_source,source))).transpose()?.unwrap_or((json!({"title":meeting.title,"note":"","tags":[]}),0,false,None,"estimated".into(),"legacy".into()))
        } else {
            (
                json!({"title":meeting.title,"note":"","tags":[]}),
                0,
                false,
                None,
                "estimated".into(),
                "legacy".into(),
            )
        };
        let duration = meeting.segments.iter().map(|s| s.duration_ms).sum::<u64>();
        let transcript = self.desktop_current_artifact(id, Stage::Transcribe)?;
        // A summary is current only when its own copied/produced transcript is current.
        let summary = self
            .desktop_current_artifact(id, Stage::Summary)?
            .filter(|summary| Some(*summary) == transcript);
        let active = self.desktop_busy(id)?;
        let latest = tasks.last();
        let status = if !self.session_sealed(id)? {
            "recording"
        } else if active {
            if latest.is_some_and(|t| t.stage == Stage::Summary) {
                "summarizing"
            } else {
                "queued"
            }
        } else if latest.is_some_and(|t| self.cancel_requested(t.id).unwrap_or(false)) {
            "cancelled"
        } else if latest.is_some_and(|t| t.status == TaskStatus::Failed) {
            "failed"
        } else if latest.is_some_and(|t| t.status == TaskStatus::Blocked) {
            "blocked"
        } else if summary.is_some() {
            "completed"
        } else if transcript.is_some() {
            "transcript_committed"
        } else if duration == 0 {
            "empty"
        } else {
            "unknown"
        };
        Ok(
            json!({"meetingId":id,"metadata":metadata,"revision":revision,"startedAtMs":started.or(Some(meeting.sealed_at_ms-duration as i64)),"sealedAtMs":meeting.sealed_at_ms,"timeSource":time_source,"durationMs":duration,"source":source,"status":status,"trashed":trashed,"capabilities":{"edit":!active,"trash":!active&&!trashed,"restore":trashed&&!active,"reprocess":!active&&!trashed&&duration>0,"reason":if active{Some("busy")}else{None}},"currentTranscriptTaskId":transcript,"currentSummaryTaskId":summary}),
        )
    }
    fn session_sealed(&self, id: Uuid) -> Result<bool> {
        let has: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='segmented_sessions')",
            [],
            |r| r.get(0),
        )?;
        if !has {
            return Ok(self.meeting(id)?.ok_or_else(missing)?.sealed_at_ms > 0);
        }
        Ok(self
            .conn
            .query_row(
                "SELECT sealed FROM segmented_sessions WHERE session=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(self.meeting(id)?.ok_or_else(missing)?.sealed_at_ms > 0))
    }
    pub fn desktop_busy(&self, id: Uuid) -> Result<bool> {
        if !self.session_sealed(id)? {
            return Ok(true);
        }
        let busy:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE session=?1 AND status IN ('queued','transcribing','transcript_committed','summarizing'))",[id.to_string()],|r|r.get(0))?;
        if busy {
            return Ok(true);
        }
        if self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='segment_jobs')",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            // Segment and archive ownership is represented by their persistent leases.
            for progress in self.segment_progress(id)? {
                if matches!(progress.status.as_str(), "queued" | "running") {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    pub fn desktop_list(&self, request: &dto::MeetingListRequest) -> Result<Value> {
        let mut rows = Vec::new();
        let query = request.filter.query.to_lowercase();
        for meeting in self.meetings()? {
            let row = self.desktop_row(meeting.session_id)?;
            let visibility = serde_json::to_value(request.filter.visibility)?;
            if visibility == "active" && row["trashed"] == true
                || visibility == "trash" && row["trashed"] == false
            {
                continue;
            }
            if !request.filter.sources.is_empty()
                && !serde_json::to_value(&request.filter.sources)?
                    .as_array()
                    .unwrap()
                    .contains(&row["source"])
            {
                continue;
            }
            if !request.filter.statuses.is_empty()
                && !serde_json::to_value(&request.filter.statuses)?
                    .as_array()
                    .unwrap()
                    .contains(&row["status"])
            {
                continue;
            }
            if let Some(date) = &request.filter.date {
                let start = row["startedAtMs"].as_i64().unwrap_or(0);
                if start < date.from_utc_ms || start >= date.to_utc_ms_exclusive {
                    continue;
                }
            }
            let scope = serde_json::to_value(request.filter.scope)?;
            let mut haystack = if scope == "content" {
                row["metadata"]["title"].as_str().unwrap_or("").to_owned()
            } else {
                format!(
                    "{} {} {}",
                    row["metadata"]["title"].as_str().unwrap_or(""),
                    row["metadata"]["note"].as_str().unwrap_or(""),
                    row["metadata"]["tags"]
                )
            };
            if !query.is_empty() && scope != "title" {
                for (field, stage) in [
                    ("currentTranscriptTaskId", Stage::Transcribe),
                    ("currentSummaryTaskId", Stage::Summary),
                ] {
                    if let Some(id) = row[field].as_str() {
                        if self.has_desktop_schema()? {
                            let text: Option<String> = self
                                .conn
                                .query_row(
                                    "SELECT content FROM desktop_search WHERE task=?1 AND stage=?2",
                                    params![id, stage_name(stage)],
                                    |r| r.get(0),
                                )
                                .optional()?;
                            haystack.push_str(text.as_deref().unwrap_or(""));
                        } else if let Ok(bytes) =
                            self.read_artifact(Uuid::parse_str(id).map_err(|_| missing())?, stage)
                        {
                            haystack.push_str(&String::from_utf8_lossy(&bytes));
                        }
                    }
                }
            }
            if scope == "title" {
                haystack = row["metadata"]["title"].as_str().unwrap_or("").to_owned();
            }
            if !haystack.to_lowercase().contains(&query) {
                continue;
            }
            rows.push(row);
        }
        let sort = serde_json::to_value(request.filter.sort)?;
        rows.sort_by(|a, b| {
            let order = if sort == "duration_asc" {
                a["durationMs"].as_i64().cmp(&b["durationMs"].as_i64())
            } else if sort == "duration_desc" {
                b["durationMs"].as_i64().cmp(&a["durationMs"].as_i64())
            } else if sort == "started_asc" {
                a["startedAtMs"].as_i64().cmp(&b["startedAtMs"].as_i64())
            } else {
                b["startedAtMs"].as_i64().cmp(&a["startedAtMs"].as_i64())
            };
            order.then_with(|| a["meetingId"].as_str().cmp(&b["meetingId"].as_str()))
        });
        let revision = hash_bytes(&serde_json::to_vec(
            &json!({"filter":request.filter,"sort":request.filter.sort,"rows":rows}),
        )?);
        if request
            .expected_revision
            .as_ref()
            .is_some_and(|r| r != &revision)
        {
            return Err(changed());
        }
        let offset = decode_cursor(request.cursor.as_deref(), &revision)?;
        let total = rows.len();
        let end = (offset + request.page_size as usize).min(total);
        if offset > total {
            return Err(changed());
        }
        Ok(
            json!({"items":rows[offset..end],"nextCursor":if end<total{Some(format!("{revision}:{end}"))}else{None},"revision":revision,"search":{"complete":true,"indexed":total,"pending":0},"total":total}),
        )
    }
    pub fn desktop_update(
        &mut self,
        id: Uuid,
        expected: i64,
        metadata: &dto::MeetingMetadata,
    ) -> Result<Value> {
        let row = self.desktop_row(id)?;
        if row["revision"] != expected {
            return Err(changed());
        }
        if self.desktop_busy(id)? {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "meeting has active work",
            ));
        }
        let tx = self.conn.write_transaction()?;
        let current: Option<i64> = tx
            .query_row(
                "SELECT revision FROM desktop_metadata WHERE meeting=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if current.unwrap_or(0) != expected {
            return Err(changed());
        }
        tx.execute("INSERT INTO desktop_metadata(meeting,revision,document,trashed) VALUES(?1,1,?2,0) ON CONFLICT(meeting) DO UPDATE SET revision=revision+1,document=excluded.document",params![id.to_string(),encode(metadata)?])?;
        tx.commit()?;
        self.desktop_row(id)
    }
    pub fn desktop_trash(
        &mut self,
        meetings: &[dto::MeetingRevision],
        trashed: bool,
    ) -> Result<Value> {
        let mut originals = BTreeMap::new();
        for item in meetings {
            let id = Uuid::parse_str(&item.meeting_id).map_err(|_| missing())?;
            let row = self.desktop_row(id)?;
            if row["revision"] != item.expected_revision {
                return Err(changed());
            }
            if self.desktop_busy(id)? {
                return Err(OmmError::new(
                    ErrorCode::Conflict,
                    false,
                    "meeting has active work",
                ));
            }
            if originals.insert(id, row).is_some() {
                return Err(OmmError::invalid("duplicate meeting"));
            }
        }
        let tx = self.conn.write_transaction()?;
        for (id, row) in &originals {
            let revision: Option<i64> = tx
                .query_row(
                    "SELECT revision FROM desktop_metadata WHERE meeting=?1",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            if revision.unwrap_or(0) != row["revision"].as_i64().unwrap() {
                return Err(changed());
            }
            tx.execute("INSERT INTO desktop_metadata(meeting,revision,document,trashed) VALUES(?1,1,?2,?3) ON CONFLICT(meeting) DO UPDATE SET revision=revision+1,trashed=excluded.trashed",params![id.to_string(),encode(&row["metadata"])?,trashed])?;
        }
        tx.commit()?;
        Ok(
            json!({"meetings":originals.keys().map(|id|self.desktop_row(*id)).collect::<Result<Vec<_>>>()?}),
        )
    }
    pub fn desktop_processing_busy(&self) -> Result<bool> {
        Ok(processing_busy(&self.conn)? || self.active_audio_readers()?)
    }
    /// The same IMMEDIATE writer boundary as every worker claim closes the final-check race.
    pub fn desktop_begin_idle_shutdown(&mut self, instance: Uuid) -> Result<()> {
        let paths = self.paths.clone();
        let tx = self.conn.write_transaction()?;
        let recording: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM settings WHERE key='recording')",
            [],
            |r| r.get(0),
        )?;
        if recording
            || processing_busy(&tx)?
            || Self::active_audio_readers_at(&paths)?
            || service_stopping(&tx)?
        {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "recording or processing became active before shutdown",
            ));
        }
        tx.execute(
            "INSERT INTO settings(key,document) VALUES('desktop_service_stopping',?1)",
            [encode(&instance)?],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn desktop_abort_idle_shutdown(&self, instance: Uuid) -> Result<()> {
        self.conn.execute(
            "DELETE FROM settings WHERE key='desktop_service_stopping' AND document=?1",
            [encode(&instance)?],
        )?;
        Ok(())
    }
    fn desktop_task_work_available(&self, task: &Task) -> Result<bool> {
        let trashed = self
            .conn
            .query_row(
                "SELECT trashed FROM desktop_metadata WHERE meeting=?1",
                [task.session_id.to_string()],
                |r| r.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false);
        let other = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE session=?1 AND id!=?2 AND status IN ('queued','transcribing','transcript_committed','summarizing'))",params![task.session_id.to_string(),task.id.to_string()],|r|r.get::<_,bool>(0))?;
        Ok(!trashed && !other)
    }
    pub fn desktop_retry(&mut self, id: Uuid) -> Result<Value> {
        if self.conn.is_autocommit() {
            return Err(OmmError::invalid(
                "desktop task mutation requires an admitted transaction",
            ));
        }
        let task = self.task(id)?;
        if self.cancel_requested(id)? || !self.desktop_task_work_available(&task)? {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "restore meeting and settle other attempts before retry",
            ));
        }
        self.retry(id)?;
        self.retry_segment_failures(task.session_id)?;
        self.desktop_task(id)
    }
    pub fn desktop_requeue(&mut self, id: Uuid, config: &ConfigSnapshot) -> Result<Value> {
        if self.conn.is_autocommit() {
            return Err(OmmError::invalid(
                "desktop task mutation requires an admitted transaction",
            ));
        }
        let task = self.task(id)?;
        if self.desktop_task(id)?["canRequeue"] != true
            || !self.desktop_task_work_available(&task)?
        {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "only settled cancelled tasks in restored idle meetings can requeue",
            ));
        }
        let meeting = self.meeting(task.session_id)?.ok_or_else(missing)?;
        let next = self.enqueue(&meeting, config, true)?;
        self.desktop_task(next.id)
    }
    pub fn desktop_task(&self, id: Uuid) -> Result<Value> {
        let task = self.task(id)?;
        let cancelled = self.cancel_requested(id)?;
        let progress = self.segment_progress(task.session_id)?;
        let (retry_count, next_retry): (u32, Option<i64>) = self
            .conn
            .query_row(
                "SELECT count,next_at FROM retries WHERE task=?1 AND stage=?2",
                params![id.to_string(), stage_name(task.stage)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .unwrap_or((0, None));
        let settled: bool = if cancelled {
            self.conn.query_row(
                "SELECT settled FROM desktop_cancellations WHERE task=?1",
                [id.to_string()],
                |r| r.get(0),
            )?
        } else {
            false
        };
        let active = matches!(
            task.status,
            TaskStatus::Queued
                | TaskStatus::Transcribing
                | TaskStatus::TranscriptCommitted
                | TaskStatus::Summarizing
        );
        let work_available = self.desktop_task_work_available(&task)?;
        let status = if settled {
            "cancelled"
        } else {
            status_name(task.status)
        };
        Ok(
            json!({"taskId":id,"meetingId":task.session_id,"attempt":task.attempt,"generationId":id,"stage":stage_name(task.stage),"status":status,"cancelRequested":cancelled,"completedSegments":progress.iter().filter(|p|p.status=="done").count(),"totalSegments":progress.len(),"retryCount":retry_count,"nextRetryAtMs":next_retry,"error":task.error.as_ref().map(crate::application::error),"canRetry":work_available&&!cancelled&&matches!(task.status,TaskStatus::Blocked|TaskStatus::Failed|TaskStatus::Interrupted),"canCancel":active&&!cancelled,"canRequeue":work_available&&cancelled&&settled}),
        )
    }
    pub fn desktop_tasks(&self, payload: &Value) -> Result<Value> {
        let mut rows = Vec::new();
        for task in self.tasks()?.iter().rev() {
            let row = self.desktop_task(task.id)?;
            if payload["meetingId"].is_string() && row["meetingId"] != payload["meetingId"] {
                continue;
            }
            if payload["statuses"]
                .as_array()
                .is_some_and(|s| !s.is_empty() && !s.contains(&row["status"]))
            {
                continue;
            }
            rows.push(row);
        }
        crate::application::page(rows, payload)
    }
    pub fn desktop_versions(&self, meeting: Uuid) -> Result<Vec<Value>> {
        let row = self.desktop_row(meeting)?;
        let mut versions = Vec::new();
        for task in self
            .tasks()?
            .iter()
            .filter(|t| t.session_id == meeting)
            .rev()
        {
            let config: String = self.conn.query_row(
                "SELECT config FROM tasks WHERE id=?1",
                [task.id.to_string()],
                |r| r.get(0),
            )?;
            let config: ConfigSnapshot = decode(&config)?;
            for stage in [Stage::Transcribe, Stage::Summary] {
                let content = self.read_artifact(task.id, stage).ok();
                let partial = if stage == Stage::Transcribe && content.is_none() {
                    self.desktop_partial_artifact(task.id)?
                } else {
                    None
                };
                let bytes = content
                    .as_deref()
                    .or_else(|| partial.as_ref().map(|(bytes, _)| bytes.as_slice()));
                let current = if stage == Stage::Transcribe {
                    row["currentTranscriptTaskId"] == task.id.to_string()
                } else {
                    row["currentSummaryTaskId"] == task.id.to_string()
                        && (row["currentTranscriptTaskId"] == task.id.to_string()
                            || task.transcript_source.as_ref().is_some_and(|s| {
                                row["currentTranscriptTaskId"] == s.task_id.to_string()
                            }))
                };
                if bytes.is_none() && stage != task.stage {
                    continue;
                }
                versions.push(json!({"artifact":{"meetingId":meeting,"taskId":task.id,"stage":stage_name(stage)},"attempt":task.attempt,"status":if content.is_some(){"committed"}else if partial.is_some(){"partial"}else{"missing"},"taskStatus":self.desktop_task(task.id)?["status"],"sha256":bytes.map(hash_bytes),"byteLength":bytes.map(<[u8]>::len),"createdAtMs":null,"sourceTranscriptTaskId":task.transcript_source.as_ref().map(|s|s.task_id).or((stage==Stage::Summary).then_some(task.id)),"sourceTranscriptHash":task.transcript_source.as_ref().map(|s|s.sha256.clone()).or_else(||(stage==Stage::Summary).then(||self.read_artifact(task.id,Stage::Transcribe).ok().map(|b|hash_bytes(&b))).flatten()),"modelVersion":config.asr.model_version,"configHash":task.config_hash,"current":current}));
            }
        }
        Ok(versions)
    }
    pub fn desktop_cancel(&mut self, id: Uuid) -> Result<Value> {
        let task = self.task(id)?;
        if task.status == TaskStatus::Completed {
            return self.desktop_task(id);
        }
        if self.cancel_requested(id)? {
            return self.desktop_task(id);
        }
        let tx = self.conn.write_transaction()?;
        tx.execute(
            "INSERT INTO desktop_cancellations VALUES(?1,?2,0)",
            params![id.to_string(), chrono::Utc::now().timestamp_millis()],
        )?;
        let live:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND lease IS NOT NULL) OR EXISTS(SELECT 1 FROM segment_jobs WHERE session=?2 AND lease IS NOT NULL)",params![id.to_string(),task.session_id.to_string()],|r|r.get(0))?;
        if !live {
            let mut task = task.clone();
            task.status = TaskStatus::Blocked;
            task.error = None;
            update_task(&tx, &task)?;
            tx.execute("UPDATE segment_jobs SET status='failed',error=NULL WHERE session=?1 AND status='queued'",[task.session_id.to_string()])?;
            tx.execute(
                "UPDATE desktop_cancellations SET settled=1 WHERE task=?1",
                [id.to_string()],
            )?;
        }
        tx.commit()?;
        self.desktop_task(id)
    }
    pub fn settle_cancellation(&mut self, id: Uuid) -> Result<()> {
        if !self.cancel_requested(id)? {
            return Ok(());
        }
        let task = self.task(id)?;
        let tx = self.conn.write_transaction()?;
        let mut task = task;
        task.status = TaskStatus::Blocked;
        task.error = None;
        update_task(&tx, &task)?;
        tx.execute("UPDATE segment_jobs SET status='failed',lease=NULL,expires=NULL,error=NULL WHERE session=?1 AND status IN ('running','queued')",[task.session_id.to_string()])?;
        tx.execute(
            "UPDATE desktop_cancellations SET settled=1 WHERE task=?1",
            [id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn desktop_import_metadata(
        &self,
        id: Uuid,
        metadata: &dto::MeetingMetadata,
        start: i64,
        hash: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO desktop_metadata VALUES(?1,1,?2,0,?3,'exact','imported')",
            params![id.to_string(), encode(metadata)?, start],
        )?;
        self.conn.execute(
            "INSERT INTO desktop_imports VALUES(?1,?2)",
            params![hash, id.to_string()],
        )?;
        Ok(())
    }
    pub fn desktop_imported(&self, id: Uuid) -> Result<bool> {
        if !self.has_desktop_schema()? {
            return Ok(false);
        }
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM desktop_imports WHERE meeting=?1)",
            [id.to_string()],
            |r| r.get(0),
        )?)
    }
    pub fn desktop_import_duplicate(&self, hash: &str) -> Result<Option<Uuid>> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT meeting FROM desktop_imports WHERE hash=?1",
                [hash],
                |r| r.get(0),
            )
            .optional()?;
        id.map(|id| Uuid::parse_str(&id).map_err(|_| missing()))
            .transpose()
    }
    pub fn cancel_requested(&self, id: Uuid) -> Result<bool> {
        if !self.has_desktop_schema()? {
            return Ok(false);
        }
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM desktop_cancellations WHERE task=?1)",
            [id.to_string()],
            |r| r.get(0),
        )?)
    }
}
pub fn decode_cursor(cursor: Option<&str>, revision: &str) -> Result<usize> {
    match cursor {
        None => Ok(0),
        Some(value) => {
            let (hash, index) = value.split_once(':').ok_or_else(changed)?;
            if hash != revision {
                return Err(changed());
            }
            index.parse().map_err(|_| changed())
        }
    }
}

impl Store {
    /// Import lock must be held first. No create/migration: fence all DB writers while
    /// checking references and removing only a verified uncommitted promotion.
    pub(crate) fn cleanup_unreferenced_import(
        paths: &Paths,
        meeting: Uuid,
        directory: &std::path::Path,
        cleanup: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let db = paths.state.join("state.sqlite");
        crate::core::paths::validate_private_dir(&paths.state)?;
        crate::core::paths::validate_private_file(&db)?;
        if db.canonicalize()? != db {
            return Err(OmmError::invalid(
                "import cleanup database is not canonical",
            ));
        }
        let mut conn = Connection::open_with_flags(
            db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let id = meeting.to_string();
        let referenced: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1) OR EXISTS(SELECT 1 FROM tasks WHERE session=?1) OR EXISTS(SELECT 1 FROM desktop_imports WHERE meeting=?1)",
            [&id], |r| r.get(0),
        )?;
        if referenced {
            return Ok(());
        }
        fn refers(value: &Value, id: &str, directory: &std::path::Path) -> bool {
            match value {
                Value::String(s) => s == id || std::path::Path::new(s).starts_with(directory),
                Value::Array(a) => a.iter().any(|v| refers(v, id, directory)),
                Value::Object(o) => o.values().any(|v| refers(v, id, directory)),
                _ => false,
            }
        }
        // Include other meetings' paths and persistent request results, not just
        // this UUID: a committed/replayable alias must never lose its audio.
        for sql in [
            "SELECT document FROM meetings",
            "SELECT segments FROM tasks",
            "SELECT audio FROM segment_jobs",
            "SELECT result FROM desktop_requests",
        ] {
            let mut stmt = tx.prepare(sql)?;
            for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
                if refers(&serde_json::from_str(&row?)?, &id, directory) {
                    return Ok(());
                }
            }
        }
        let directory_referenced: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE directory=?1 OR substr(directory,1,length(?1)+1)=?1||'/')",
            [directory.to_string_lossy().as_ref()], |r| r.get(0),
        )?;
        if !directory_referenced {
            cleanup()?;
        }
        // No database changes. Drop rolls back the read/check fence on every path.
        Ok(())
    }
}

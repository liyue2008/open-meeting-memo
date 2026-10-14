//! Independent durable segment jobs. The parent task waits without owning a lease.
use super::super::paths::validate_private_dir;
use super::*;

#[derive(Debug, Clone)]
pub struct ClaimedSegment {
    pub sequence: u32,
    pub offset_frames: u64,
    pub frame_count: u64,
    pub context: TaskContext,
    pub audio: AudioSegment,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SegmentProgress {
    pub sequence: u32,
    pub offset_frames: u64,
    pub frame_count: u64,
    pub status: String,
    pub retries: u32,
    pub error: Option<OmmError>,
    pub activity: Option<AsrWorkerEvent>,
}
impl Store {
    pub(super) fn initialize_segments(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS segmented_sessions(session TEXT PRIMARY KEY REFERENCES meetings(id),parent TEXT NOT NULL REFERENCES tasks(id),sealed INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS segment_jobs(session TEXT NOT NULL REFERENCES segmented_sessions(session),sequence INTEGER NOT NULL,offset_frames INTEGER NOT NULL,frames INTEGER NOT NULL,audio TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'queued',lease TEXT,expires INTEGER,retries INTEGER NOT NULL DEFAULT 0,next_at INTEGER NOT NULL DEFAULT 0,error TEXT,staging TEXT,destination TEXT,hash TEXT,PRIMARY KEY(session,sequence));
        CREATE TABLE IF NOT EXISTS segment_activity(lease TEXT PRIMARY KEY,document TEXT NOT NULL);")?;
        Ok(())
    }
    pub fn begin_segmented(
        &mut self,
        meeting: &SealedMeeting,
        config: &ConfigSnapshot,
    ) -> Result<Task> {
        validate_private_dir(&meeting.directory)?;
        if !meeting.segments.is_empty()
            || meeting.sealed_at_ms != 0
            || meeting.directory.canonicalize()? != meeting.directory
            || !meeting
                .directory
                .starts_with(self.paths.data.join("meetings").canonicalize()?)
        {
            return Err(OmmError::invalid("active meeting metadata is invalid"));
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let task = Task {
            id: Uuid::new_v4(),
            session_id: meeting.session_id,
            attempt: 1,
            stage: Stage::Transcribe,
            status: TaskStatus::Queued,
            input_hash: hash_bytes(b"[]"),
            config_hash: hash_bytes(encode(config)?.as_bytes()),
            summary_metadata: None,
            transcript_source: None,
            error: None,
        };
        tx.execute(
            "INSERT INTO meetings VALUES(?1,?2)",
            params![meeting.session_id.to_string(), encode(meeting)?],
        )?;
        tx.execute("INSERT INTO tasks(id,session,attempt,input_hash,config_hash,document,stage,status,directory,config,segments) VALUES(?1,?2,1,?3,?4,?5,'transcribe','queued',?6,?7,'[]')",params![task.id.to_string(),task.session_id.to_string(),task.input_hash,task.config_hash,encode(&task)?,meeting.directory.to_str(),encode(config)?])?;
        tx.execute(
            "INSERT INTO segmented_sessions VALUES(?1,?2,0)",
            params![meeting.session_id.to_string(), task.id.to_string()],
        )?;
        tx.commit()?;
        Ok(task)
    }
    pub fn admit_segment(
        &mut self,
        session: Uuid,
        sequence: u32,
        audio: &AudioSegment,
    ) -> Result<()> {
        let config: String = self.conn.query_row("SELECT config FROM tasks JOIN segmented_sessions ON tasks.id=parent WHERE segmented_sessions.session=?1", [session.to_string()], |r| r.get(0))?;
        let config: ConfigSnapshot = decode(&config)?;
        self.admit_segment_with_audio_config(session, sequence, audio, &config.audio)
    }
    /// Audio encoding settings freeze at first admission, including work not yet encoded.
    pub fn admit_segment_with_audio_config(
        &mut self,
        session: Uuid,
        sequence: u32,
        audio: &AudioSegment,
        config: &AudioConfig,
    ) -> Result<()> {
        config.validate()?;
        if let Some(capture) = &audio.capture {
            let descriptor = PathBuf::from(format!("{}.sealed.json", audio.path.display()));
            controlled_file(&self.paths.data, &descriptor)?;
            let file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&descriptor)?;
            let mut bytes = Vec::new();
            file.take(4097).read_to_end(&mut bytes)?;
            if bytes.len() > 4096 || hash_bytes(&bytes) != capture.descriptor_sha256 {
                return Err(OmmError::new(
                    ErrorCode::Corrupt,
                    false,
                    "capture descriptor hash differs during admission",
                ));
            }
            let doc: serde_json::Value = serde_json::from_slice(&bytes)?;
            if doc["schema_version"] != 1
                || doc["session_id"] != session.to_string()
                || doc["instance_id"] != capture.instance_id.to_string()
                || doc["sequence"] != capture.sequence
                || doc["offset_frames"] != capture.offset_frames
                || doc["frame_count"] != capture.frame_count
                || doc["sample_rate"] != capture.sample_rate
                || doc["path"] != audio.path.to_string_lossy().as_ref()
                || doc["legacy_wav"].as_bool().unwrap_or(false) != capture.legacy_wav
                || (capture.legacy_wav
                    && (capture.sequence != 0
                        || capture.offset_frames != 0
                        || doc["temporary"] != false))
            {
                return Err(conflict());
            }
        }
        let old: Option<String> = self
            .conn
            .query_row(
                "SELECT audio FROM segment_jobs WHERE session=?1 AND sequence=?2",
                params![session.to_string(), sequence],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(old) = old {
            return if old == encode(audio)? {
                if std::fs::symlink_metadata(&audio.path).is_ok()
                    && hash_file(&audio.path)? != audio.sha256
                {
                    return Err(OmmError::new(
                        ErrorCode::Corrupt,
                        false,
                        "admitted segment source changed",
                    ));
                }
                Ok(())
            } else {
                Err(conflict())
            };
        }
        let dir:String=self.conn.query_row("SELECT directory FROM tasks JOIN segmented_sessions ON tasks.id=parent WHERE segmented_sessions.session=?1",[session.to_string()],|r|r.get(0))?;
        controlled_file(&PathBuf::from(dir), &audio.path)?;
        if hash_file(&audio.path)? != audio.sha256 {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "segment source hash mismatch",
            ));
        }
        let reader = hound::WavReader::open(&audio.path)
            .map_err(|_| OmmError::invalid("segment WAV is invalid"))?;
        let spec = reader.spec();
        let frames = u64::from(reader.duration());
        if spec.channels != 2
            || spec.sample_rate != 48000
            || spec.bits_per_sample != 16
            || spec.sample_format != hound::SampleFormat::Int
            || frames == 0
            || (frames > 1800 * 48000
                && audio
                    .capture
                    .as_ref()
                    .is_some_and(|capture| !capture.legacy_wav))
            || audio.duration_ms != frames * 1000 / 48000
        {
            return Err(OmmError::invalid("segment format or duration is invalid"));
        }
        if let Some(capture) = &audio.capture {
            if capture.session_id != session
                || capture.sequence != sequence
                || capture.frame_count != frames
                || capture.sample_rate != 48000
                || capture.descriptor_sha256.len() != 64
                || !capture
                    .descriptor_sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit())
            {
                return Err(OmmError::invalid(
                    "capture descriptor identity or frame metadata differs",
                ));
            }
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT audio FROM segment_jobs WHERE session=?1 AND sequence=?2",
                params![session.to_string(), sequence],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            if existing != encode(audio)? {
                return Err(conflict());
            }
            return Ok(());
        }
        let (sealed,count,offset):(bool,u32,u64)=tx.query_row("SELECT sealed,(SELECT COUNT(*) FROM segment_jobs WHERE session=?1),(SELECT COALESCE(SUM(frames),0) FROM segment_jobs WHERE session=?1) FROM segmented_sessions WHERE session=?1",[session.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        if let Some(capture) = &audio.capture {
            if capture.offset_frames != offset {
                return Err(OmmError::invalid("capture descriptor frame origin differs"));
            }
            if sequence > 0 {
                let previous: String = tx.query_row(
                    "SELECT audio FROM segment_jobs WHERE session=?1 AND sequence=0",
                    [session.to_string()],
                    |r| r.get(0),
                )?;
                let previous: AudioSegment = decode(&previous)?;
                if previous
                    .capture
                    .as_ref()
                    .is_some_and(|p| p.instance_id != capture.instance_id)
                {
                    return Err(conflict());
                }
            }
        }
        if sealed || count != sequence {
            return Err(OmmError::invalid(
                "segment sequence is not contiguous or meeting sealed",
            ));
        }
        let persisted_offset = audio
            .capture
            .as_ref()
            .map(|c| c.offset_frames)
            .unwrap_or(offset);
        tx.execute("INSERT INTO segment_jobs(session,sequence,offset_frames,frames,audio) VALUES(?1,?2,?3,?4,?5)",params![session.to_string(),sequence,persisted_offset,frames,encode(audio)?])?;
        tx.execute(
            "INSERT INTO segment_archives(session,sequence,bitrate) VALUES(?1,?2,?3)",
            params![
                session.to_string(),
                sequence,
                config.opus_bitrate_kbps * 1000
            ],
        )?;
        let doc: String = tx.query_row(
            "SELECT document FROM meetings WHERE id=?1",
            [session.to_string()],
            |r| r.get(0),
        )?;
        let mut meeting: SealedMeeting = decode(&doc)?;
        meeting.segments.push(audio.clone());
        tx.execute(
            "UPDATE meetings SET document=?2 WHERE id=?1",
            params![session.to_string(), encode(&meeting)?],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn seal_segmented(&mut self, meeting: &SealedMeeting) -> Result<Task> {
        for (sequence, audio) in meeting.segments.iter().enumerate() {
            self.admit_segment(meeting.session_id, sequence as u32, audio)?;
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (parent, sealed): (String, bool) = tx.query_row(
            "SELECT parent,sealed FROM segmented_sessions WHERE session=?1",
            [meeting.session_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if sealed {
            let old: String = tx.query_row(
                "SELECT document FROM meetings WHERE id=?1",
                [meeting.session_id.to_string()],
                |r| r.get(0),
            )?;
            if old != encode(meeting)? {
                return Err(conflict());
            }
        }
        let count: usize = tx.query_row(
            "SELECT COUNT(*) FROM segment_jobs WHERE session=?1",
            [meeting.session_id.to_string()],
            |r| r.get(0),
        )?;
        if count != meeting.segments.len() {
            return Err(conflict());
        }
        let mut task: Task = decode(&tx.query_row::<String, _, _>(
            "SELECT document FROM tasks WHERE id=?1",
            [&parent],
            |r| r.get(0),
        )?)?;
        task.input_hash = hash_bytes(encode(&meeting.segments)?.as_bytes());
        tx.execute(
            "UPDATE meetings SET document=?2 WHERE id=?1",
            params![meeting.session_id.to_string(), encode(meeting)?],
        )?;
        tx.execute(
            "UPDATE tasks SET document=?2,input_hash=?3,segments=?4 WHERE id=?1",
            params![
                parent,
                encode(&task)?,
                task.input_hash,
                encode(&meeting.segments)?
            ],
        )?;
        tx.execute(
            "UPDATE segmented_sessions SET sealed=1 WHERE session=?1",
            [meeting.session_id.to_string()],
        )?;
        tx.commit()?;
        Ok(task)
    }
    pub fn record_segment_progress(
        &self,
        context: &TaskContext,
        event: &AsrWorkerEvent,
    ) -> Result<()> {
        self.conn.execute("INSERT INTO segment_activity SELECT ?1,?2 WHERE EXISTS(SELECT 1 FROM segment_jobs WHERE lease=?1 AND expires>?3 AND status='running') ON CONFLICT(lease) DO UPDATE SET document=excluded.document",params![context.lease.token.to_string(),encode(event)?,chrono::Utc::now().timestamp_millis()])?;
        Ok(())
    }
    pub fn has_segmented_session(&self, id: Uuid) -> Result<bool> {
        if !self.has_segment_tables()? {
            return Ok(false);
        }
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM segmented_sessions WHERE session=?1)",
            [id.to_string()],
            |r| r.get(0),
        )?)
    }
    fn has_segment_tables(&self) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='segmented_sessions')",
            [],
            |r| r.get(0),
        )?)
    }
    pub fn is_segmented_parent(&self, id: Uuid) -> Result<bool> {
        if !self.has_segment_tables()? {
            return Ok(false);
        }
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM segmented_sessions WHERE parent=?1)",
            [id.to_string()],
            |r| r.get(0),
        )?)
    }
    // Imported summaries and forced attempts retain the original session's frame clock;
    // pre-rolling WAV meetings retain their millisecond sums.
    pub(super) fn transcript_duration(&self, task: &Task, audio: &[AudioSegment]) -> Result<u64> {
        let mut origin = task.clone();
        let mut visited = std::collections::HashSet::new();
        loop {
            if !visited.insert(origin.id)
                || origin.session_id != task.session_id
                || origin.input_hash != task.input_hash
            {
                return Err(conflict());
            }
            if self.has_segmented_session(origin.session_id)? {
                let (offset, frames): (u64, u64) = self.conn.query_row(
                    "SELECT offset_frames,frames FROM segment_jobs WHERE session=?1 ORDER BY sequence DESC LIMIT 1",
                    [origin.session_id.to_string()], |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                return offset
                    .checked_add(frames)
                    .and_then(|v| v.checked_mul(1000))
                    .map(|v| v / 48000)
                    .ok_or_else(|| OmmError::invalid("audio duration overflow"));
            }
            match origin.transcript_source {
                Some(source) => origin = self.task(source.task_id)?,
                None => {
                    return audio
                        .iter()
                        .try_fold(0u64, |v, s| v.checked_add(s.duration_ms))
                        .ok_or_else(|| OmmError::invalid("audio duration overflow"))
                }
            }
        }
    }
    pub fn claim_segment(&mut self, now: i64, lease_ms: i64) -> Result<Option<ClaimedSegment>> {
        if lease_ms <= 0 {
            return Err(OmmError::invalid("segment lease must be positive"));
        }
        self.recover_segment_commits()?;
        self.repair_segment_checkpoints()?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if crate::core::resources::CpuBudget::current().shared_encoder_slot && tx.query_row("SELECT EXISTS(SELECT 1 FROM segment_archives WHERE lease IS NOT NULL AND expires>?1)",[now],|r|r.get::<_,bool>(0))?{return Ok(None);}
        tx.execute("UPDATE segment_jobs SET status='queued',lease=NULL,expires=NULL WHERE status='running' AND expires<=?1",[now])?;
        let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM segment_jobs WHERE lease IS NOT NULL AND expires>?1) OR EXISTS(SELECT 1 FROM tasks WHERE lease IS NOT NULL AND expires>?1 AND stage='transcribe')",[now],|r|r.get(0))?;
        if busy {
            return Ok(None);
        }
        type Row = (String, u32, u64, u64, String, String, String, String);
        let row:Option<Row>=tx.query_row("SELECT j.session,j.sequence,j.offset_frames,j.frames,j.audio,t.id,t.directory,t.config FROM segment_jobs j JOIN segmented_sessions s ON s.session=j.session JOIN tasks t ON t.id=s.parent WHERE j.status='queued' AND j.next_at<=?1 ORDER BY j.rowid LIMIT 1",[now],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional()?;
        let Some((session, sequence, offset_frames, frame_count, audio, parent, dir, config)) = row
        else {
            return Ok(None);
        };
        let lease = Lease {
            token: Uuid::new_v4(),
            expires_at_ms: now
                .checked_add(lease_ms)
                .ok_or_else(|| OmmError::invalid("lease overflow"))?,
        };
        tx.execute("UPDATE segment_jobs SET status='running',lease=?3,expires=?4,error=NULL WHERE session=?1 AND sequence=?2",params![session,sequence,lease.token.to_string(),lease.expires_at_ms])?;
        tx.commit()?;
        let work = self
            .paths
            .cache
            .join("segment-tasks")
            .join(lease.token.to_string());
        private_dir(&work)?;
        Ok(Some(ClaimedSegment {
            sequence,
            offset_frames,
            frame_count,
            audio: decode(&audio)?,
            context: TaskContext {
                session_id: Uuid::parse_str(&session).map_err(|_| conflict())?,
                task_id: Uuid::parse_str(&parent).map_err(|_| conflict())?,
                attempt: 1,
                lease,
                meeting_dir: PathBuf::from(dir),
                work_dir: work,
                config: decode(&config)?,
            },
        }))
    }
    pub fn renew_segment(&mut self, c: &mut ClaimedSegment, now: i64, lease_ms: i64) -> Result<()> {
        if lease_ms <= 0 {
            return Err(OmmError::invalid("segment lease must be positive"));
        }
        let expires = now
            .checked_add(lease_ms)
            .ok_or_else(|| OmmError::invalid("lease overflow"))?;
        let changed=self.conn.execute("UPDATE segment_jobs SET expires=?4 WHERE session=?1 AND sequence=?2 AND lease=?3 AND expires>?5 AND status='running'",params![c.context.session_id.to_string(),c.sequence,c.context.lease.token.to_string(),expires,now])?;
        if changed != 1 {
            return Err(conflict());
        }
        c.context.lease.expires_at_ms = expires;
        Ok(())
    }
    pub fn commit_segment(&mut self, c: &ClaimedSegment, t: &Transcript, now: i64) -> Result<()> {
        let duration = c.frame_count * 1000 / 48000;
        if t.schema_version != 1
            || t.model_version != c.context.config.asr.model_version
            || t.duration_ms != duration
            || t.segments
                .iter()
                .any(|s| s.start_ms >= s.end_ms || s.end_ms > duration || s.text.trim().is_empty())
        {
            return Err(OmmError::invalid(
                "segment transcript identity or bounds invalid",
            ));
        }
        if c.audio.path.exists() {
            controlled_file(&c.context.meeting_dir, &c.audio.path)?;
            if hash_file(&c.audio.path)? != c.audio.sha256 {
                return Err(conflict());
            }
        } else {
            let receipt = self
                .archive_for_audio(c.context.session_id, &c.audio)?
                .ok_or_else(conflict)?;
            for file in receipt.files {
                controlled_file(&c.context.meeting_dir, &file.path)?;
                if hash_file(&file.path)? != file.sha256
                    || file.frames != c.frame_count
                    || file.decoded_frames != c.frame_count
                {
                    return Err(conflict());
                }
            }
        }
        let dir = c
            .context
            .meeting_dir
            .join(format!("segments-{}", c.context.task_id));
        private_dir(&dir)?;
        let staging = dir.join(format!("{}.tmp", c.context.lease.token));
        let dest = dir.join(format!("{:04}-{}.json", c.sequence, c.context.lease.token));
        let bytes = serde_json::to_vec(t)?;
        write_private_new(&staging, &bytes)?;
        sync_dir(&dir)?;
        let changed=self.conn.execute("UPDATE segment_jobs SET status='committing',staging=?4,destination=?5,hash=?6 WHERE session=?1 AND sequence=?2 AND lease=?3 AND expires>?7 AND status='running'",params![c.context.session_id.to_string(),c.sequence,c.context.lease.token.to_string(),staging.to_str(),dest.to_str(),hash_bytes(&bytes),now])?;
        if changed != 1 {
            fs::remove_file(staging)?;
            return Err(conflict());
        }
        self.recover_segment_commits()?;
        let mut parent = self.task(c.context.task_id)?;
        parent.status = TaskStatus::Queued;
        parent.error = None;
        self.conn.execute(
            "UPDATE tasks SET document=?2,status='queued' WHERE id=?1",
            params![parent.id.to_string(), encode(&parent)?],
        )?;
        Ok(())
    }
    fn repair_segment_checkpoints(&mut self) -> Result<()> {
        let rows = {
            let mut q=self.conn.prepare("SELECT j.session,j.sequence,j.frames,j.destination,j.hash,t.config FROM segment_jobs j JOIN segmented_sessions s ON s.session=j.session JOIN tasks t ON t.id=s.parent WHERE j.status='done' AND t.stage='transcribe'")?;
            let rows = q.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (session, sequence, frames, destination, hash, config) in rows {
            let valid = (|| -> Result<bool> {
                let path = PathBuf::from(destination);
                controlled_file(&self.paths.data, &path)?;
                let bytes = fs::read(path)?;
                let t: Transcript = serde_json::from_slice(&bytes)?;
                let cfg: ConfigSnapshot = decode(&config)?;
                Ok(hash_bytes(&bytes) == hash
                    && t.schema_version == 1
                    && t.model_version == cfg.asr.model_version
                    && t.duration_ms == frames * 1000 / 48000
                    && t.segments.iter().all(|s| {
                        s.start_ms < s.end_ms
                            && s.end_ms <= t.duration_ms
                            && !s.text.trim().is_empty()
                    }))
            })();
            if !matches!(valid, Ok(true)) {
                self.conn.execute("UPDATE segment_jobs SET status='queued',staging=NULL,destination=NULL,hash=NULL,lease=NULL,expires=NULL,next_at=0 WHERE session=?1 AND sequence=?2 AND status='done'",params![session,sequence])?;
            }
        }
        Ok(())
    }
    fn recover_segment_commits(&mut self) -> Result<()> {
        let rows = {
            let mut stmt=self.conn.prepare("SELECT session,sequence,staging,destination,hash FROM segment_jobs WHERE status='committing'")?;
            let r = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?;
            r.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (session, sequence, tmp, dest, hash) in rows {
            let tx = self
                .conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let status: String = tx.query_row(
                "SELECT status FROM segment_jobs WHERE session=?1 AND sequence=?2",
                params![session, sequence],
                |r| r.get(0),
            )?;
            if status != "committing" {
                continue;
            }
            let dest = PathBuf::from(dest);
            let tmp = PathBuf::from(tmp);
            if !dest.exists() {
                controlled_file(&self.paths.data, &tmp)?;
                if hash_file(&tmp)? != hash {
                    return Err(conflict());
                }
                fs::rename(&tmp, &dest)?;
                sync_dir(dest.parent().unwrap())?;
            }
            controlled_file(&self.paths.data, &dest)?;
            if hash_file(&dest)? != hash {
                return Err(conflict());
            }
            tx.execute("UPDATE segment_jobs SET status='done',lease=NULL,expires=NULL,error=NULL WHERE session=?1 AND sequence=?2",params![session,sequence])?;
            tx.commit()?;
            if tmp.exists() {
                controlled_file(&self.paths.data, &tmp)?;
                fs::remove_file(tmp)?;
            }
        }
        Ok(())
    }
    pub fn fail_segment(&mut self, c: &ClaimedSegment, error: &OmmError, now: i64) -> Result<()> {
        let retries: u32 = self.conn.query_row(
            "SELECT retries FROM segment_jobs WHERE session=?1 AND sequence=?2",
            params![c.context.session_id.to_string(), c.sequence],
            |r| r.get(0),
        )?;
        let status = if error.retryable && retries < 3 {
            "queued"
        } else {
            "failed"
        };
        let changed=self.conn.execute("UPDATE segment_jobs SET status=?4,error=?5,retries=retries+1,next_at=?6,lease=NULL,expires=NULL WHERE session=?1 AND sequence=?2 AND lease=?3 AND expires>?7 AND status='running'",params![c.context.session_id.to_string(),c.sequence,c.context.lease.token.to_string(),status,encode(error)?,now+1000*(1_i64<<retries.min(3)),now])?;
        if changed != 1 {
            return Err(conflict());
        }
        let mut parent = self.task(c.context.task_id)?;
        parent.status = TaskStatus::Failed;
        parent.error = Some(error.clone());
        self.conn.execute(
            "UPDATE tasks SET document=?2,status='failed' WHERE id=?1",
            params![parent.id.to_string(), encode(&parent)?],
        )?;
        Ok(())
    }
    pub fn segment_progress(&self, session: Uuid) -> Result<Vec<SegmentProgress>> {
        if !self.has_segmented_session(session)? {
            return Ok(vec![]);
        }
        let mut stmt=self.conn.prepare("SELECT sequence,offset_frames,frames,status,retries,error,(SELECT document FROM segment_activity WHERE segment_activity.lease=segment_jobs.lease) FROM segment_jobs WHERE session=?1 ORDER BY sequence")?;
        let rows = stmt.query_map([session.to_string()], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, u32>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })?;
        rows.map(|r| {
            let (sequence, offset_frames, frame_count, status, retries, error, activity) = r?;
            Ok(SegmentProgress {
                sequence,
                offset_frames,
                frame_count,
                status,
                retries,
                error: error.as_deref().map(decode).transpose()?,
                activity: activity.as_deref().map(decode).transpose()?,
            })
        })
        .collect()
    }
    pub fn partial_transcript(&self, session: Uuid) -> Result<Transcript> {
        let config:String=self.conn.query_row("SELECT config FROM tasks JOIN segmented_sessions ON tasks.id=parent WHERE segmented_sessions.session=?1",[session.to_string()],|r|r.get(0))?;
        let cfg: ConfigSnapshot = decode(&config)?;
        let mut result = Transcript {
            schema_version: 1,
            model_version: cfg.asr.model_version,
            duration_ms: 0,
            segments: vec![],
        };
        let mut stmt=self.conn.prepare("SELECT offset_frames,frames,destination,hash,status FROM segment_jobs WHERE session=?1 ORDER BY sequence")?;
        let rows = stmt.query_map([session.to_string()], |r| {
            Ok((
                r.get::<_, u64>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        for row in rows {
            let (offset, frames, dest, hash, status) = row?;
            result.duration_ms = (offset + frames) * 1000 / 48000;
            if status != "done" {
                continue;
            }
            let path = PathBuf::from(dest.ok_or_else(conflict)?);
            controlled_file(&self.paths.data, &path)?;
            let bytes = fs::read(&path)?;
            if Some(hash_bytes(&bytes)) != hash {
                return Err(conflict());
            }
            let t: Transcript = serde_json::from_slice(&bytes)?;
            if t.model_version != result.model_version || t.duration_ms != frames * 1000 / 48000 {
                return Err(conflict());
            }
            for mut s in t.segments {
                s.start_ms += offset * 1000 / 48000;
                s.end_ms = (s.end_ms + offset * 1000 / 48000).min(result.duration_ms);
                result.segments.push(s);
            }
        }
        result.segments.sort_by_key(|s| (s.start_ms, s.end_ms));
        Ok(result)
    }
}
impl Store {
    /// An explicit retry resumes only failed segments/archives, never valid checkpoints.
    pub fn retry_segment_failures(&mut self, session: Uuid) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let count=tx.execute("UPDATE segment_jobs SET status='queued',retries=0,next_at=0,error=NULL WHERE session=?1 AND status='failed'",[session.to_string()])?;
        tx.execute("UPDATE segment_archives SET status='queued',retries=0,next_at=0,error=NULL WHERE session=?1 AND status='failed'",[session.to_string()])?;
        if count > 0 {
            let doc:String=tx.query_row("SELECT document FROM tasks JOIN segmented_sessions s ON tasks.id=s.parent WHERE s.session=?1",[session.to_string()],|r|r.get(0))?;
            let mut task: Task = decode(&doc)?;
            task.status = TaskStatus::Queued;
            task.error = None;
            update_task(&tx, &task)?;
        }
        tx.commit()?;
        Ok(())
    }
}

impl Store {
    pub fn record_asr_inputs(
        &mut self,
        context: &TaskContext,
        document: &serde_json::Value,
    ) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS asr_inputs(task TEXT NOT NULL REFERENCES tasks(id),lease TEXT NOT NULL,document TEXT NOT NULL,hash TEXT NOT NULL,PRIMARY KEY(task,lease))")?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let alive:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND lease=?2 AND expires>?3) OR EXISTS(SELECT 1 FROM segment_jobs j JOIN segmented_sessions s ON j.session=s.session WHERE s.parent=?1 AND j.lease=?2 AND j.expires>?3 AND j.status='running')",params![context.task_id.to_string(),context.lease.token.to_string(),chrono::Utc::now().timestamp_millis()],|r|r.get(0))?;
        if !alive {
            return Err(conflict());
        }
        let body = encode(document)?;
        let hash = super::hash_bytes(body.as_bytes());
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT document,hash FROM asr_inputs WHERE task=?1 AND lease=?2",
                params![context.task_id.to_string(), context.lease.token.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((old, old_hash)) = existing {
            if old != body || old_hash != hash {
                return Err(conflict());
            }
        } else {
            tx.execute(
                "INSERT INTO asr_inputs VALUES(?1,?2,?3,?4)",
                params![
                    context.task_id.to_string(),
                    context.lease.token.to_string(),
                    body,
                    hash
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn recorded_asr_inputs(&self, task: Uuid) -> Result<Option<serde_json::Value>> {
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='asr_inputs')",
            [],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(None);
        }
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT document,hash FROM asr_inputs WHERE task=?1 ORDER BY rowid DESC LIMIT 1",
                [task.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(body, hash)| {
            if super::hash_bytes(body.as_bytes()) != hash {
                return Err(OmmError::new(
                    ErrorCode::Corrupt,
                    false,
                    "ASR input checkpoint hash differs",
                ));
            }
            decode(&body)
        })
        .transpose()
    }
}

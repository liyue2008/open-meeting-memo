//! Archive leases are independent of transcription and never block capture.
use super::segments::ClaimedSegment;
use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CapturedWavProvenance {
    schema_version: u32,
    session_id: Uuid,
    instance_id: Uuid,
    #[serde(default)]
    temporary: bool,
    #[serde(default)]
    legacy_wav: bool,
    sequence: u32,
    offset_frames: u64,
    frame_count: u64,
    sample_rate: u32,
    path: PathBuf,
    #[serde(
        default,
        deserialize_with = "crate::core::contracts::deserialize_capture_clock"
    )]
    capture_clock: Option<CaptureClock>,
}
impl Store {
    pub(super) fn initialize_archives(&self) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE IF NOT EXISTS segment_archives(session TEXT NOT NULL,sequence INTEGER NOT NULL,status TEXT NOT NULL DEFAULT 'queued',lease TEXT,expires INTEGER,receipt TEXT,retries INTEGER NOT NULL DEFAULT 0,next_at INTEGER NOT NULL DEFAULT 0,error TEXT,cleaned INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(session,sequence),FOREIGN KEY(session,sequence) REFERENCES segment_jobs(session,sequence));INSERT OR IGNORE INTO segment_archives(session,sequence) SELECT session,sequence FROM segment_jobs;")?;
        let has_bitrate: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('segment_archives') WHERE name='bitrate')", [], |r| r.get(0))?;
        if !has_bitrate {
            // Before configurable encoding, every admitted job used the fixed 48 kbps policy.
            self.conn.execute_batch(
                "ALTER TABLE segment_archives ADD COLUMN bitrate INTEGER NOT NULL DEFAULT 48000",
            )?;
        }
        Ok(())
    }
    pub fn claim_archive(&mut self, now: i64, lease_ms: i64) -> Result<Option<ClaimedSegment>> {
        if lease_ms <= 0 {
            return Err(OmmError::invalid("archive lease invalid"));
        }
        let tx = self.conn.write_transaction()?;
        if crate::core::resources::CpuBudget::current().shared_encoder_slot {
            let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM segment_jobs WHERE lease IS NOT NULL AND expires>?1) OR EXISTS(SELECT 1 FROM tasks WHERE stage='transcribe' AND lease IS NOT NULL AND expires>?1)",[now],|r|r.get(0))?;
            if busy {
                return Ok(None);
            }
        }
        tx.execute("UPDATE segment_archives SET status='queued',lease=NULL,expires=NULL WHERE status='running' AND expires<=?1",[now])?;
        let busy: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM segment_archives WHERE lease IS NOT NULL AND expires>?1)",
            [now],
            |r| r.get(0),
        )?;
        if busy {
            return Ok(None);
        }
        type Row = (String, u32, u64, u64, String, String, String, String, u32);
        let row:Option<Row>=tx.query_row("SELECT a.session,a.sequence,j.offset_frames,j.frames,j.audio,t.id,t.directory,t.config,a.bitrate FROM segment_archives a JOIN segment_jobs j USING(session,sequence) JOIN segmented_sessions s ON s.session=a.session JOIN tasks t ON t.id=s.parent WHERE a.status='queued' AND a.next_at<=?1 ORDER BY a.rowid LIMIT 1",[now],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional()?;
        let Some((session, sequence, offset_frames, frame_count, audio, id, dir, config, bitrate)) =
            row
        else {
            return Ok(None);
        };
        let lease = Lease {
            token: Uuid::new_v4(),
            expires_at_ms: now.checked_add(lease_ms).ok_or_else(conflict)?,
        };
        tx.execute("UPDATE segment_archives SET status='running',lease=?3,expires=?4,error=NULL WHERE session=?1 AND sequence=?2",params![session,sequence,lease.token.to_string(),lease.expires_at_ms])?;
        tx.commit()?;
        let work = self
            .paths
            .cache
            .join("archive-tasks")
            .join(lease.token.to_string());
        private_dir(&work)?;
        let mut config: ConfigSnapshot = decode(&config)?;
        config.audio.opus_bitrate_kbps = bitrate / 1000;
        config.audio.validate()?;
        if bitrate % 1000 != 0 {
            return Err(conflict());
        }
        Ok(Some(ClaimedSegment {
            sequence,
            offset_frames,
            frame_count,
            audio: decode(&audio)?,
            context: TaskContext {
                session_id: Uuid::parse_str(&session).map_err(|_| conflict())?,
                task_id: Uuid::parse_str(&id).map_err(|_| conflict())?,
                attempt: 1,
                lease,
                meeting_dir: PathBuf::from(dir),
                work_dir: work,
                config,
            },
        }))
    }
    pub fn renew_archive(&mut self, c: &mut ClaimedSegment, now: i64, lease_ms: i64) -> Result<()> {
        let expires = now.checked_add(lease_ms).ok_or_else(conflict)?;
        let n=self.conn.execute("UPDATE segment_archives SET expires=?4 WHERE session=?1 AND sequence=?2 AND lease=?3 AND expires>?5 AND status='running'",params![c.context.session_id.to_string(),c.sequence,c.context.lease.token.to_string(),expires,now])?;
        if n != 1 {
            return Err(conflict());
        }
        c.context.lease.expires_at_ms = expires;
        Ok(())
    }
    /// Only a newly claimed explicit retry can reuse retained encoded media.
    pub fn retained_archive_receipt(&self, c: &ClaimedSegment) -> Result<Option<ArchiveReceipt>> {
        let raw: Option<String> = self.conn.query_row("SELECT receipt FROM segment_archives WHERE session=?1 AND sequence=?2 AND status='running' AND lease=?3 AND expires>?4",params![c.context.session_id.to_string(),c.sequence,c.context.lease.token.to_string(),chrono::Utc::now().timestamp_millis()],|r|r.get(0))?;
        raw.as_deref().map(decode).transpose()
    }
    pub fn commit_archive(
        &mut self,
        c: &ClaimedSegment,
        receipt: &ArchiveReceipt,
        now: i64,
    ) -> Result<()> {
        if receipt.schema_version != 1
            || receipt.source_sha256 != c.audio.sha256
            || receipt.files.len() != 3
        {
            return Err(OmmError::invalid("archive receipt identity invalid"));
        }
        for source in ["mix", "mic", "system"] {
            let f = receipt
                .files
                .iter()
                .find(|f| f.source == source)
                .ok_or_else(conflict)?;
            controlled_file(&c.context.meeting_dir, &f.path)?;
            let (pre_skip, tail_padding) = crate::core::opus::metadata(&f.path, c.frame_count)?;
            if hash_file(&f.path)? != f.sha256
                || f.pre_skip != pre_skip
                || f.tail_padding != tail_padding
                || f.decoded_frames != c.frame_count
                || f.frames != c.frame_count
                || f.channels != 1
                || f.sample_rate != 48000
                || f.codec != "opus"
                || f.container != "ogg"
                || f.bitrate != c.context.config.audio.opus_bitrate_kbps * 1000
            {
                return Err(OmmError::new(
                    ErrorCode::Corrupt,
                    false,
                    "archive receipt or audio identity mismatch",
                ));
            }
        }
        let n=self.conn.execute("UPDATE segment_archives SET status='done',receipt=?4,lease=NULL,expires=NULL,error=NULL WHERE session=?1 AND sequence=?2 AND lease=?3 AND expires>?5 AND status='running'",params![c.context.session_id.to_string(),c.sequence,c.context.lease.token.to_string(),encode(receipt)?,now])?;
        if n != 1 {
            return Err(conflict());
        }
        Ok(())
    }
    pub fn fail_archive(&mut self, c: &ClaimedSegment, error: &OmmError, now: i64) -> Result<()> {
        let retries: u32 = self.conn.query_row(
            "SELECT retries FROM segment_archives WHERE session=?1 AND sequence=?2",
            params![c.context.session_id.to_string(), c.sequence],
            |r| r.get(0),
        )?;
        let n=self.conn.execute("UPDATE segment_archives SET status=?4,error=?5,retries=retries+1,next_at=?6,lease=NULL,expires=NULL WHERE session=?1 AND sequence=?2 AND lease=?3 AND expires>?7 AND status='running'",params![c.context.session_id.to_string(),c.sequence,c.context.lease.token.to_string(),if error.retryable&&retries<3{"queued"}else{"failed"},encode(error)?,now+1000*(1_i64<<retries.min(3)),now])?;
        if n != 1 {
            return Err(conflict());
        }
        Ok(())
    }
    pub fn archive_for_audio(
        &self,
        session: Uuid,
        audio: &AudioSegment,
    ) -> Result<Option<ArchiveReceipt>> {
        let row:Option<(String,String)>=self.conn.query_row("SELECT a.receipt,j.audio FROM segment_archives a JOIN segment_jobs j USING(session,sequence) WHERE a.session=?1 AND a.status='done' AND json_extract(j.audio,'$.path')=?2",params![session.to_string(),audio.path.to_str()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((receipt, stored)) = row else {
            return Ok(None);
        };
        if stored != encode(audio)? {
            return Err(conflict());
        }
        let receipt: ArchiveReceipt = decode(&receipt)?;
        if receipt.source_sha256 != audio.sha256 {
            return Err(conflict());
        }
        Ok(Some(receipt))
    }
    pub fn archive_progress(&self, session: Uuid) -> Result<serde_json::Value> {
        if !self.has_segmented_session(session)? {
            return Ok(serde_json::json!([]));
        }
        let mut stmt=self.conn.prepare("SELECT sequence,status,retries,error,receipt,cleaned FROM segment_archives WHERE session=?1 ORDER BY sequence")?;
        let rows = stmt.query_map([session.to_string()], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })?;
        let values=rows.map(|r|{let(sequence,status,retries,error,receipt,cleaned)=r?;let receipt=receipt.as_deref().map(decode::<ArchiveReceipt>).transpose()?;Ok(serde_json::json!({"sequence":sequence,"status":status,"retries":retries,"error":error.as_deref().map(decode::<OmmError>).transpose()?,"audio":receipt.and_then(|r|r.files.into_iter().find(|f|f.source=="mix")),"temporaryWavRemoved":cleaned}))}).collect::<Result<Vec<_>>>()?;
        Ok(serde_json::json!(values))
    }
    pub fn cleanup_archived_sources(&mut self) -> Result<()> {
        self.cleanup_archived_sources_inner(false)
    }
    /// Quarantine only a rejected archive row; infrastructure/database errors still propagate.
    pub fn cleanup_archived_sources_for_queue(&mut self) -> Result<()> {
        self.cleanup_archived_sources_inner(true)
    }
    fn cleanup_archived_sources_inner(&mut self, quarantine: bool) -> Result<()> {
        // Optimistic admission only; candidate rows are fetched again under the exclusive lock below.
        let candidate: Option<i64> = self.conn.query_row("SELECT 1 FROM segment_jobs j JOIN segment_archives a USING(session,sequence) JOIN segmented_sessions s ON s.session=j.session WHERE s.sealed=1 AND j.status='done' AND a.status='done' AND a.cleaned=0 AND NOT EXISTS(SELECT 1 FROM tasks WHERE session=j.session AND lease IS NOT NULL) LIMIT 1", [], |r| r.get(0)).optional()?;
        if candidate.is_none() {
            return Ok(());
        }
        // Shared reader locks survive cancellation until each owned child has exited.
        let path = self.paths.state.join("audio-readers.lock");
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        super::super::paths::validate_private_file(&path)?;
        match fs2::FileExt::try_lock_exclusive(&lock) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        // Only sealed sessions: the capture helper can still replay active WAV descriptors.
        let rows = {
            let mut stmt=self.conn.prepare("SELECT j.session,j.sequence,j.audio,j.offset_frames,j.frames FROM segment_jobs j JOIN segment_archives a USING(session,sequence) JOIN segmented_sessions s ON s.session=j.session WHERE s.sealed=1 AND j.status='done' AND a.status='done' AND a.cleaned=0 AND NOT EXISTS(SELECT 1 FROM tasks WHERE session=j.session AND lease IS NOT NULL)")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, u64>(3)?,
                    r.get::<_, u64>(4)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (session, sequence, audio, offset_frames, frames) in rows {
            if let Err(error) =
                self.cleanup_archived_source(&session, sequence, &audio, offset_frames, frames)
            {
                if !quarantine {
                    return Err(error);
                }
                // Receipt and originals remain; the failed row is visible and cannot kill either lane.
                self.conn.execute("UPDATE segment_archives SET status='failed',error=?3 WHERE session=?1 AND sequence=?2 AND status='done'", params![session, sequence, encode(&error)?])?;
            }
        }
        Ok(())
    }
    fn cleanup_archived_source(
        &mut self,
        session: &str,
        sequence: u32,
        audio: &str,
        offset_frames: u64,
        frames: u64,
    ) -> Result<()> {
        let audio: AudioSegment = decode(audio)?;
        let Some(capture) = &audio.capture else {
            return Ok(());
        };
        // Queue admission alone does not make a pre-upgrade WAV disposable.
        // Only the new rolling writer publishes this immutable capture provenance.
        let mut name = audio.path.as_os_str().to_os_string();
        name.push(".sealed.json");
        let descriptor = PathBuf::from(name);
        if !descriptor.exists() {
            return Ok(());
        }
        controlled_file(&self.paths.data, &descriptor)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&descriptor)?;
        let mut bytes = Vec::new();
        file.take(4097).read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            return Err(OmmError::invalid("capture provenance exceeds limit"));
        }
        if hash_bytes(&bytes) != capture.descriptor_sha256 {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "capture provenance hash differs",
            ));
        }
        let provenance: CapturedWavProvenance = serde_json::from_slice(&bytes)?;
        if provenance.schema_version != 1
            || provenance.session_id.to_string() != session
            || provenance.instance_id != capture.instance_id
            || provenance.session_id != capture.session_id
            || provenance.sequence != sequence
            || provenance.offset_frames != offset_frames
            || provenance.frame_count != frames
            || provenance.sample_rate != 48000
            || provenance.path != audio.path
            || provenance.legacy_wav != capture.legacy_wav
            || (provenance.legacy_wav
                && (provenance.temporary || sequence != 0 || offset_frames != 0))
        {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "capture provenance identity mismatch",
            ));
        }
        // Typed diagnostics are optional for old helpers, never an authority to delete media.
        if provenance
            .capture_clock
            .as_ref()
            .is_some_and(|c| !c.valid())
        {
            return Err(OmmError::invalid("capture clock invalid"));
        }
        if !provenance.temporary || provenance.legacy_wav {
            return Ok(());
        }
        let receipt = self
            .archive_for_audio(Uuid::parse_str(session).map_err(|_| conflict())?, &audio)?
            .ok_or_else(conflict)?;
        if receipt.schema_version != 1
            || receipt.source_sha256 != audio.sha256
            || receipt.files.len() != 3
        {
            return Err(conflict());
        }
        let bitrate: u32 = self.conn.query_row(
            "SELECT bitrate FROM segment_archives WHERE session=?1 AND sequence=?2",
            params![session, sequence],
            |r| r.get(0),
        )?;
        for source in ["mix", "mic", "system"] {
            let f = receipt
                .files
                .iter()
                .find(|f| f.source == source)
                .ok_or_else(conflict)?;
            controlled_file(&self.paths.data, &f.path)?;
            let (pre_skip, tail_padding) = crate::core::opus::metadata(&f.path, frames)?;
            if hash_file(&f.path)? != f.sha256
                || f.frames != frames
                || f.decoded_frames != frames
                || f.sample_rate != 48000
                || f.channels != 1
                || f.codec != "opus"
                || f.container != "ogg"
                || f.bitrate != bitrate
                || f.pre_skip != pre_skip
                || f.tail_padding != tail_padding
            {
                return Err(conflict());
            }
        }
        if audio.path.exists() {
            controlled_file(&self.paths.data, &audio.path)?;
            if hash_file(&audio.path)? != audio.sha256 {
                return Err(conflict());
            }
            fs::remove_file(&audio.path)?;
            sync_dir(audio.path.parent().unwrap())?;
        }
        self.conn.execute(
            "UPDATE segment_archives SET cleaned=1 WHERE session=?1 AND sequence=?2",
            params![session, sequence],
        )?;
        Ok(())
    }
}
impl Store {
    /// Public audio is the mono mix; source-labelled archives remain maintenance data.
    pub fn meeting_audio(&self, session: Uuid) -> Result<Vec<ArchivedAudio>> {
        if !self.has_segmented_session(session)? {
            return Ok(vec![]);
        }
        let mut stmt=self.conn.prepare("SELECT j.sequence,j.offset_frames,a.receipt FROM segment_archives a JOIN segment_jobs j USING(session,sequence) WHERE a.session=?1 AND a.status='done' ORDER BY j.sequence")?;
        let rows = stmt.query_map([session.to_string()], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (sequence, offset, receipt) = row?;
            let receipt: ArchiveReceipt = decode(&receipt)?;
            let file = receipt
                .files
                .into_iter()
                .find(|f| f.source == "mix")
                .ok_or_else(conflict)?;
            Ok(ArchivedAudio {
                sequence,
                offset_ms: offset * 1000 / 48000,
                file,
            })
        })
        .collect()
    }
}

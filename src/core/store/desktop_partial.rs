//! A partial artifact is a verified parent/segment snapshot, never a whole-meeting commit.
use super::*;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

impl Store {
    pub fn desktop_partial_artifact(&self, task_id: Uuid) -> Result<Option<(Vec<u8>, String)>> {
        // Keep parent, segment states and hashes in one SQLite read snapshot, including read-only opens.
        let snapshot = self
            .conn
            .is_autocommit()
            .then(|| self.conn.unchecked_transaction())
            .transpose()?;
        let task = self.task(task_id)?;
        if !self.has_segmented_session(task.session_id)? {
            return Ok(None);
        }
        let parent: Option<String> = self
            .conn
            .query_row(
                "SELECT parent FROM segmented_sessions WHERE session=?1",
                [task.session_id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if parent.as_deref() != Some(&task_id.to_string())
            || self.artifact_committed(task_id, Stage::Transcribe)?
        {
            return Ok(None);
        }
        let config: String = self.conn.query_row(
            "SELECT config FROM tasks WHERE id=?1",
            [task_id.to_string()],
            |r| r.get(0),
        )?;
        let config: ConfigSnapshot = decode(&config)?;
        let mut transcript = Transcript {
            schema_version: 1,
            model_version: config.asr.model_version,
            duration_ms: 0,
            segments: vec![],
        };
        let meeting = self
            .meeting(task.session_id)?
            .ok_or_else(partial_conflict)?;
        if !meeting
            .directory
            .starts_with(self.paths.data.join("meetings"))
            || meeting.directory.canonicalize()? != meeting.directory
        {
            return Err(partial_conflict());
        }
        let directory = meeting.directory.join(format!("segments-{task_id}"));
        let mut query=self.conn.prepare("SELECT sequence,offset_frames,frames,destination,hash,status FROM segment_jobs WHERE session=?1 ORDER BY sequence")?;
        let rows = query.query_map([task.session_id.to_string()], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut segments = Vec::new();
        let mut source_bytes = 0u64;
        let mut completed = 0;
        for row in rows {
            let (sequence, offset, frames, destination, hash, status) = row?;
            transcript.duration_ms = (offset + frames) * 1000 / 48000;
            segments.push((sequence, offset, frames, hash.clone(), status.clone()));
            if status != "done" {
                continue;
            }
            let path = PathBuf::from(destination.ok_or_else(partial_conflict)?);
            crate::core::paths::validate_private_dir(&meeting.directory)?;
            crate::core::paths::validate_private_dir(&directory)?;
            controlled_file(&directory, &path)?;
            crate::core::paths::validate_private_file(&path)?;
            let mut file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)?;
            let metadata = file.metadata()?;
            source_bytes = source_bytes
                .checked_add(metadata.len())
                .ok_or_else(partial_conflict)?;
            // Bound parsing memory independently of the per-response chunk budget.
            if source_bytes > 64 * 1024 * 1024 || metadata.nlink() != 1 {
                return Err(OmmError::unavailable(
                    "partial transcript source budget or identity invalid",
                ));
            }
            let mut bytes = Vec::new();
            Read::by_ref(&mut file)
                .take(metadata.len() + 1)
                .read_to_end(&mut bytes)?;
            controlled_file(&directory, &path)?;
            let current = fs::symlink_metadata(&path)?;
            if metadata.ino() != current.ino()
                || metadata.dev() != current.dev()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o077 != 0
                || bytes.len() as u64 != metadata.len()
                || Some(hash_bytes(&bytes)) != hash
            {
                return Err(partial_conflict());
            }
            let partial: Transcript = serde_json::from_slice(&bytes)?;
            if partial.schema_version != 1
                || partial.model_version != transcript.model_version
                || partial.duration_ms != frames * 1000 / 48000
            {
                return Err(partial_conflict());
            }
            for mut line in partial.segments {
                if line.start_ms > line.end_ms || line.end_ms > partial.duration_ms {
                    return Err(partial_conflict());
                }
                line.start_ms += offset * 1000 / 48000;
                line.end_ms += offset * 1000 / 48000;
                transcript.segments.push(line);
            }
            completed += 1;
        }
        if completed == 0 {
            return Ok(None);
        }
        transcript
            .segments
            .sort_by_key(|line| (line.start_ms, line.end_ms));
        let bytes = serde_json::to_vec(&transcript)?;
        let revision = hash_bytes(&serde_json::to_vec(&(
            "partial-v1",
            task.session_id,
            task_id,
            task.stage,
            task.status,
            task.config_hash,
            task.input_hash,
            segments,
            hash_bytes(&bytes),
        ))?);
        drop(query);
        if let Some(snapshot) = snapshot {
            snapshot.commit()?;
        }
        Ok(Some((bytes, revision)))
    }
}
fn partial_conflict() -> OmmError {
    OmmError::new(
        ErrorCode::Conflict,
        false,
        "partial transcript snapshot changed or failed verification",
    )
}

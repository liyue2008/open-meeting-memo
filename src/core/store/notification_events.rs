//! Notification evidence is observational; callers must not couple writes to business transitions.
use super::*;
use crate::core::notifications::NotificationRecord;
impl Store {
    pub fn meeting(&self, session: Uuid) -> Result<Option<SealedMeeting>> {
        self.conn
            .query_row(
                "SELECT document FROM meetings WHERE id=?1",
                [session.to_string()],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|s| decode(&s))
            .transpose()
    }
    pub fn record_notification(&self, record: &NotificationRecord) -> Result<()> {
        self.set_state(
            &format!(
                "notification-event:{:020}:{}",
                record.occurred_at_ms, record.id
            ),
            record,
        )
    }
    pub fn notification_history(
        &self,
        meeting: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<NotificationRecord>> {
        if !(1..=100).contains(&limit) {
            return Err(OmmError::invalid(
                "notification history limit must be 1–100",
            ));
        }
        let mut stmt=self.conn.prepare("SELECT document FROM settings WHERE key LIKE 'notification-event:%' AND (?1 IS NULL OR json_extract(document,'$.session_id')=?1) ORDER BY substr(key,1,39) DESC,rowid DESC LIMIT ?2")?;
        let rows = stmt.query_map(params![meeting.map(|id| id.to_string()), limit], |r| {
            r.get::<_, String>(0)
        })?;
        rows.map(|s| decode(&s?)).collect()
    }
    pub fn notification_retry_state(
        &self,
        task: Uuid,
        stage: Stage,
        sequence: Option<u32>,
        retryable: bool,
    ) -> Result<(String, u32, Option<i64>)> {
        let task = self.task(task)?;
        let (status, count, next_at) =
            if let Some(sequence) = sequence {
                self.conn.query_row(
                "SELECT status,retries,next_at FROM segment_jobs WHERE session=?1 AND sequence=?2",
                params![task.session_id.to_string(), sequence],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?, Some(r.get::<_, i64>(2)?))),
            )?
            } else {
                let (count, next_at) = self
                    .conn
                    .query_row(
                        "SELECT count,next_at FROM retries WHERE task=?1 AND stage=?2",
                        params![task.id.to_string(), stage_name(stage)],
                        |r| Ok((r.get::<_, u32>(0)?, Some(r.get::<_, i64>(1)?))),
                    )
                    .optional()?
                    .unwrap_or((0, None));
                (status_name(task.status).to_owned(), count, next_at)
            };
        let state = if status == "blocked" {
            "blocked"
        } else if !retryable {
            "not_retryable"
        } else if status == "queued" || status == "transcript_committed" {
            "queued"
        } else if (sequence.is_some() && status == "failed") || count >= 3 {
            "exhausted"
        } else if status == "failed" {
            "pending"
        } else {
            "unknown"
        };
        Ok((
            state.into(),
            if sequence.is_some() {
                count.min(3)
            } else {
                count
            },
            if state == "queued" { next_at } else { None },
        ))
    }
}

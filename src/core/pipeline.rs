use super::{
    contracts::*,
    notifications::{NotificationEvent, NotificationSink},
    store::Store,
};
/// One stage with renewable ownership; committing the transcript automatically queues summary.
pub async fn run_one(
    store: &mut Store,
    asr: &dyn AsrAdapter,
    pi: &dyn SummaryAdapter,
    lease_ms: i64,
) -> Result<Option<Task>> {
    run_one_notifying(store, asr, pi, lease_ms, &NotificationSink::default()).await
}
pub async fn run_one_notifying(
    store: &mut Store,
    asr: &dyn AsrAdapter,
    pi: &dyn SummaryAdapter,
    lease_ms: i64,
    notifications: &NotificationSink,
) -> Result<Option<Task>> {
    let now = chrono::Utc::now().timestamp_millis();
    let Some(mut claim) = store.claim(now, lease_ms)? else {
        return Ok(None);
    };
    let context = claim.context.clone();
    let input = if claim.stage == Stage::Summary {
        Some(
            store
                .read_artifact(claim.task.id, Stage::Transcribe)
                .and_then(|b| Ok(serde_json::from_slice::<Transcript>(&b)?)),
        )
    } else {
        None
    };
    enum Output {
        Transcript(Transcript),
        Summary(Summary),
    }
    let stage = claim.stage;
    let segments = claim.segments.clone();
    let model_lock = if stage == Stage::Transcribe {
        Some(store.model_read_lock())
    } else {
        None
    };
    let operation = async move {
        match stage {
            Stage::Transcribe => {
                let _lock = model_lock.unwrap()?;
                Ok(Output::Transcript(
                    asr.transcribe(AsrRequest { context, segments }).await?,
                ))
            }
            Stage::Summary => Ok(Output::Summary(
                pi.summarize(SummaryRequest {
                    context,
                    transcript: input.unwrap()?,
                })
                .await?,
            )),
        }
    };
    // The future owns a context copy; renew the durable lease without restarting the adapter.
    let mut operation = Box::pin(operation);
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_millis(
        (lease_ms as u64 / 3).max(1),
    ));
    let result: Result<Output> = loop {
        tokio::select! {
            output=&mut operation => break output,
            _=heartbeat.tick() => { store.renew(&mut claim,chrono::Utc::now().timestamp_millis(),lease_ms)?; }
        }
    };
    drop(operation);
    let result = result.and_then(|out| match out {
        Output::Transcript(t) => {
            store.commit_transcript(&claim, &t, chrono::Utc::now().timestamp_millis())
        }
        Output::Summary(s) => {
            store.commit_summary(&claim, &s, chrono::Utc::now().timestamp_millis())
        }
    });
    if let Err(e) = result {
        store.fail(&claim, &e, chrono::Utc::now().timestamp_millis())?;
        notifications.emit(
            if stage == Stage::Transcribe {
                NotificationEvent::AsrFailed
            } else {
                NotificationEvent::SummaryFailed
            },
            claim.task.id,
            claim.context.attempt,
        );
    } else if stage == Stage::Summary {
        notifications.emit(
            NotificationEvent::SummaryReady,
            claim.task.id,
            claim.context.attempt,
        );
    }
    // Completed disposable work is removed; original audio and committed artifacts never are.
    if claim.context.work_dir.exists() {
        std::fs::remove_dir_all(&claim.context.work_dir)?;
    }
    Ok(Some(store.task(claim.task.id)?))
}

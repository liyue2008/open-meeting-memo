use super::{
    contracts::*,
    notifications::{NotificationEvent, NotificationSink},
    store::{StageAttemptError, StageErrorPhase, Store},
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
    run_one_lane(store, asr, pi, lease_ms, notifications, None).await
}
pub async fn run_asr_one_notifying(
    store: &mut Store,
    asr: &dyn AsrAdapter,
    pi: &dyn SummaryAdapter,
    lease_ms: i64,
    notifications: &NotificationSink,
) -> Result<Option<Task>> {
    run_one_lane(
        store,
        asr,
        pi,
        lease_ms,
        notifications,
        Some(Stage::Transcribe),
    )
    .await
}
pub async fn run_summary_one_notifying(
    store: &mut Store,
    asr: &dyn AsrAdapter,
    pi: &dyn SummaryAdapter,
    lease_ms: i64,
    notifications: &NotificationSink,
) -> Result<Option<Task>> {
    run_one_lane(
        store,
        asr,
        pi,
        lease_ms,
        notifications,
        Some(Stage::Summary),
    )
    .await
}
async fn run_one_lane(
    store: &mut Store,
    asr: &dyn AsrAdapter,
    pi: &dyn SummaryAdapter,
    lease_ms: i64,
    notifications: &NotificationSink,
    lane: Option<Stage>,
) -> Result<Option<Task>> {
    let now = chrono::Utc::now().timestamp_millis();
    if lane != Some(Stage::Summary) {
        if let Some(mut segment) = store.claim_segment(now, lease_ms)? {
            let started = std::time::Instant::now();
            let context = segment.context.clone();
            let audio = segment.audio.clone();
            let model_lock = store.model_read_lock()?;
            let mut operation = Box::pin(asr.transcribe(AsrRequest {
                source_kind: None,
                context,
                segments: vec![audio],
            }));
            let mut heartbeat = tokio::time::interval(std::time::Duration::from_millis(
                (lease_ms as u64 / 3).clamp(1, 250),
            ));
            let result = loop {
                tokio::select! {output=&mut operation=>break output,_=heartbeat.tick()=>{if store.cancel_requested(segment.context.task_id)? {break Err(OmmError::new(ErrorCode::Unavailable,false,"task cancellation requested"));}store.renew_segment(&mut segment,chrono::Utc::now().timestamp_millis(),lease_ms)?;}}
            };
            drop(operation);
            drop(model_lock);
            let now = chrono::Utc::now().timestamp_millis();
            let failure_phase = if result.is_ok() {
                StageErrorPhase::Commit
            } else {
                StageErrorPhase::Adapter
            };
            let result = match result {
                Err(e) => {
                    record_stage_error_best_effort(
                        store,
                        &StageAttemptError::new(
                            &segment.context,
                            Stage::Transcribe,
                            StageErrorPhase::Adapter,
                            Some(segment.sequence),
                            vec![segment.audio.sha256.clone()],
                            started.elapsed().as_millis() as u64,
                            &e,
                        ),
                    );
                    Err(e)
                }
                Ok(t) => {
                    let result = store.commit_segment(&segment, &t, now);
                    if let Err(e) = &result {
                        record_stage_error_best_effort(
                            store,
                            &StageAttemptError::new(
                                &segment.context,
                                Stage::Transcribe,
                                StageErrorPhase::Commit,
                                Some(segment.sequence),
                                vec![segment.audio.sha256.clone()],
                                started.elapsed().as_millis() as u64,
                                e,
                            ),
                        );
                    }
                    result
                }
            };
            match result {
                Ok(()) => {}
                Err(e) => {
                    store.fail_segment(&segment, &e, now)?;
                    let retry = store
                        .notification_retry_state(
                            segment.context.task_id,
                            Stage::Transcribe,
                            Some(segment.sequence),
                            e.retryable,
                        )
                        .ok();
                    notifications.emit_failure(
                        NotificationEvent::AsrFailed,
                        &segment.context,
                        Some(segment.sequence),
                        &e,
                        failure_phase,
                        retry,
                    );
                }
            }
            asr.drain().await?;
            store.settle_cancellation(segment.context.task_id)?;
            if segment.context.work_dir.exists() {
                std::fs::remove_dir_all(&segment.context.work_dir)?;
            }
            return Ok(Some(store.task(segment.context.task_id)?));
        }
    }
    let claimed = match lane {
        Some(Stage::Transcribe) => store.claim_asr(now, lease_ms)?,
        Some(Stage::Summary) => match store.claim_summary(now, lease_ms)? {
            some @ Some(_) => some,
            None => store.claim_ready_aggregate(now, lease_ms)?,
        },
        None => store.claim(now, lease_ms)?,
    };
    let Some(mut claim) = claimed else {
        return Ok(None);
    };
    let started = std::time::Instant::now();
    let context = claim.context.clone();
    let partial = if claim.stage == Stage::Transcribe && store.is_segmented_parent(claim.task.id)? {
        Some(store.partial_transcript(claim.task.session_id))
    } else {
        None
    };
    let audio_decode = claim.stage == Stage::Transcribe && partial.is_none();
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
    let model_lock = if audio_decode {
        Some(store.model_read_lock())
    } else {
        None
    };
    let source_kind = store
        .desktop_imported(claim.context.session_id)?
        .then_some(AudioSource::Mix);
    let operation = async move {
        match stage {
            Stage::Transcribe => {
                let _lock = model_lock.transpose()?;
                Ok(Output::Transcript(match partial {
                    Some(t) => t?,
                    None => {
                        asr.transcribe(AsrRequest {
                            source_kind,
                            context,
                            segments,
                        })
                        .await?
                    }
                }))
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
        (lease_ms as u64 / 3).clamp(1, 250),
    ));
    let result: Result<Output> = loop {
        tokio::select! {
            output=&mut operation => break output,
            _=heartbeat.tick() => { if store.cancel_requested(claim.task.id)? {break Err(OmmError::new(ErrorCode::Unavailable,false,"task cancellation requested"));} store.renew(&mut claim,chrono::Utc::now().timestamp_millis(),lease_ms)?; }
        }
    };
    drop(operation);
    if let Err(e) = &result {
        record_stage_error_best_effort(
            store,
            &StageAttemptError::new(
                &claim.context,
                stage,
                StageErrorPhase::Adapter,
                None,
                claim.segments.iter().map(|a| a.sha256.clone()).collect(),
                started.elapsed().as_millis() as u64,
                e,
            ),
        );
    }
    let failure_phase = if result.is_ok() {
        StageErrorPhase::Commit
    } else {
        StageErrorPhase::Adapter
    };
    let result = result.and_then(|out| match out {
        Output::Transcript(t) => {
            let result = store.commit_transcript(&claim, &t, chrono::Utc::now().timestamp_millis());
            if let Err(e) = &result {
                record_stage_error_best_effort(
                    store,
                    &StageAttemptError::new(
                        &claim.context,
                        stage,
                        StageErrorPhase::Commit,
                        None,
                        claim.segments.iter().map(|a| a.sha256.clone()).collect(),
                        started.elapsed().as_millis() as u64,
                        e,
                    ),
                );
            }
            result
        }
        Output::Summary(s) => {
            let result = store.commit_summary(&claim, &s, chrono::Utc::now().timestamp_millis());
            if let Err(e) = &result {
                record_stage_error_best_effort(
                    store,
                    &StageAttemptError::new(
                        &claim.context,
                        stage,
                        StageErrorPhase::Commit,
                        None,
                        claim.segments.iter().map(|a| a.sha256.clone()).collect(),
                        started.elapsed().as_millis() as u64,
                        e,
                    ),
                );
            }
            result
        }
    });
    if let Err(e) = result {
        store.fail(&claim, &e, chrono::Utc::now().timestamp_millis())?;
        // Daemon lanes settle their one bounded retry before freezing this failed lease's notice.
        // Manual run_one leaves retry pending, as it does not own an automatic retry loop.
        if lane.is_some() {
            let failed = store.task(claim.task.id)?;
            store.schedule_retry(&failed, chrono::Utc::now().timestamp_millis())?;
        }
        let retry = store
            .notification_retry_state(claim.task.id, stage, None, e.retryable)
            .ok();
        notifications.emit_failure(
            if stage == Stage::Transcribe {
                NotificationEvent::AsrFailed
            } else {
                NotificationEvent::SummaryFailed
            },
            &claim.context,
            None,
            &e,
            failure_phase,
            retry,
        );
    } else if stage == Stage::Summary {
        notifications.emit_task(NotificationEvent::SummaryReady, &claim.context);
    }
    // Completed disposable work is removed only after owned ASR children exited.
    if audio_decode {
        asr.drain().await?;
    }
    pi.drain().await?;
    store.settle_cancellation(claim.task.id)?;
    if claim.context.work_dir.exists() {
        std::fs::remove_dir_all(&claim.context.work_dir)?;
    }
    if let Some(parent) = claim.context.work_dir.parent() {
        if parent.exists() && std::fs::read_dir(parent)?.next().is_none() {
            match std::fs::remove_dir(parent) {
                Ok(()) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(Some(store.task(claim.task.id)?))
}

// Diagnostics are observers: losing this evidence must not change fail/commit handling.
fn record_stage_error_best_effort(store: &Store, error: &StageAttemptError) {
    if let Err(observer_error) = store.record_stage_error(error) {
        let mut evidence = error.clone();
        evidence
            .input_hashes
            .retain(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
        // This independent stderr record contains typed metadata only, never either error message.
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr().lock(),
            "warning: stage_error_diagnostic_unavailable observer_code={:?} evidence={}",
            observer_error.code,
            serde_json::json!(evidence)
        );
    }
}

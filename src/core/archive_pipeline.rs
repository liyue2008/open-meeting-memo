use crate::{
    adapters::asr::codec::AudioCodec,
    core::{contracts::*, store::Store},
};
/// One low-priority encoder, independent of the sole ASR worker.
pub async fn run_one(store: &mut Store, codec: &Result<AudioCodec>, lease_ms: i64) -> Result<bool> {
    let now = chrono::Utc::now().timestamp_millis();
    let Some(mut claim) = store.claim_archive(now, lease_ms)? else {
        return Ok(false);
    };
    let result = match codec {
        Err(e) => Err(e.clone()),
        Ok(codec) => {
            let context = claim.context.clone();
            let audio = claim.audio.clone();
            let sequence = claim.sequence;
            let mut operation = Box::pin(codec.archive(
                &context,
                &audio,
                sequence,
                context.config.audio.opus_bitrate_kbps * 1000,
            ));
            let mut heartbeat = tokio::time::interval(std::time::Duration::from_millis(
                (lease_ms as u64 / 3).max(1),
            ));
            let result = loop {
                tokio::select! {output=&mut operation=>break output,_=heartbeat.tick()=>{store.renew_archive(&mut claim,chrono::Utc::now().timestamp_millis(),lease_ms)?;}}
            };
            drop(operation);
            codec.drain().await?;
            result
        }
    };
    let now = chrono::Utc::now().timestamp_millis();
    match result.and_then(|receipt| store.commit_archive(&claim, &receipt, now)) {
        Ok(()) => {}
        Err(e) => store.fail_archive(&claim, &e, now)?,
    }
    if claim.context.work_dir.exists() {
        std::fs::remove_dir_all(&claim.context.work_dir)?;
    }
    store.cleanup_archived_sources_for_queue()?;
    Ok(true)
}

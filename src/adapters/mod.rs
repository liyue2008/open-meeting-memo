// Final product statically registers both native components.
#[cfg(feature = "asr")]
#[rustfmt::skip]
pub mod asr;
#[cfg(feature = "capture")]
#[rustfmt::skip]
pub mod capture;
pub mod meeting;
pub mod pi;

use crate::core::contracts::*;
use async_trait::async_trait;
/// Missing worker configuration is an explicit task dependency failure.
pub struct UnavailableAsr;
#[async_trait]
impl AsrAdapter for UnavailableAsr {
    async fn transcribe(&self, _: AsrRequest) -> Result<Transcript> {
        Err(OmmError::unavailable(
            "native ASR worker executable is not configured",
        ))
    }
}

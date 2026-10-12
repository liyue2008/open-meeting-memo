use crate::core::contracts::*;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

pub const MODEL_MANIFEST: &str = include_str!("../../../packaging/asr/models.json");
pub const MODEL_VERSION: &str = "sherpa-onnx-1.13.2-fire-red-asr-large-zh_en-2025-02-16-int8";

pub fn sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
    }
    Ok(hex::encode(digest.finalize()))
}
/// Read-only: never download or repair missing models. Hashes are compiled in.
pub fn verify_models(cfg: &AsrConfig) -> Result<()> {
    if cfg.model_version != MODEL_VERSION {
        return Err(OmmError::invalid(
            "unsupported ASR model version; use the pinned model manifest",
        ));
    }
    let manifest: serde_json::Value = serde_json::from_str(MODEL_MANIFEST)?;
    for (path, asset, name) in [
        (&cfg.encoder_path, 0, "encoder.int8.onnx"),
        (&cfg.decoder_path, 0, "decoder.int8.onnx"),
        (&cfg.tokens_path, 0, "tokens.txt"),
        (&cfg.vad_path, 1, "silero_vad.onnx"),
    ] {
        if !path.is_absolute() || !path.is_file() {
            return Err(OmmError::unavailable(
                "pinned local ASR model file missing; explicit model download is required",
            ));
        }
        let expected = manifest["assets"][asset]["files"][name]
            .as_str()
            .ok_or_else(|| OmmError::invalid("invalid compiled model manifest"))?;
        if sha256(path)? != expected {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "local ASR model SHA256 mismatch; explicit repair is required",
            ));
        }
    }
    Ok(())
}

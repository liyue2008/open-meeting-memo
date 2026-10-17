//! Native dialog selections are copied through a pinned descriptor; JavaScript only receives handles.
use super::{desktop_contracts::MeetingMetadata, profile::ProfileOwner};
use crate::core::{
    contracts::*,
    paths::*,
    store::{hash_bytes, hash_file, Store},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use uuid::Uuid;
mod promotion;
mod staging;
const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    pub handle: String,
    pub generation: Uuid,
    pub profile_id: String,
    pub display_name: String,
    pub byte_length: u64,
    pub format: String,
    pub duration_ms: u64,
    pub channels: u16,
    pub sample_rate: u32,
    pub expires_at_ms: i64,
    pub source_hash: String,
    pub mix_hash: String,
    pub meeting_id: Uuid,
}
fn root(paths: &Paths) -> PathBuf {
    paths.cache.join("desktop-imports")
}
fn directory(paths: &Paths, handle: &str) -> Result<PathBuf> {
    if handle.len() != 32 || !handle.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(OmmError::invalid("invalid import handle"));
    }
    Ok(root(paths).join(handle))
}
fn wav(path: &Path) -> Result<(u64, u16, u32)> {
    let reader =
        hound::WavReader::open(path).map_err(|_| OmmError::invalid("unsupported WAV container"))?;
    let spec = reader.spec();
    let duration = reader.duration() as u64 * 1000 / spec.sample_rate.max(1) as u64;
    if !matches!(
        (spec.sample_format, spec.bits_per_sample),
        (hound::SampleFormat::Int, 16 | 24 | 32) | (hound::SampleFormat::Float, 32)
    ) || !(1..=2).contains(&spec.channels)
        || !(8000..=96000).contains(&spec.sample_rate)
        || duration == 0
        || duration > 28_800_000
        || reader.len() as u64 != reader.duration() as u64 * spec.channels as u64
    {
        return Err(OmmError::invalid(
            "WAV requires aligned PCM16/24/32 or float32 mono/stereo at 8–96 kHz, at most 8 hours",
        ));
    }
    Ok((duration, spec.channels, spec.sample_rate))
}
fn opus(path: &Path) -> Result<(u64, u16, u32)> {
    let mut file = File::open(path)?;
    let mut serial = None;
    let mut pre_skip = 0;
    let mut channels = 0;
    let mut granule = 0;
    let mut first = true;
    loop {
        let mut header = [0; 27];
        let n = file.read(&mut header[..1])?;
        if n == 0 {
            break;
        }
        file.read_exact(&mut header[1..])?;
        if &header[..4] != b"OggS" || header[4] != 0 {
            return Err(OmmError::invalid("invalid Ogg Opus header"));
        }
        let stream = u32::from_le_bytes(header[14..18].try_into().unwrap());
        if serial.is_some_and(|s| s != stream) {
            return Err(OmmError::invalid(
                "chained or multiplexed Opus imports are not supported",
            ));
        }
        serial = Some(stream);
        let mut lacing = vec![0; header[26] as usize];
        file.read_exact(&mut lacing)?;
        let mut body = vec![0; lacing.iter().map(|v| *v as usize).sum()];
        file.read_exact(&mut body)?;
        if first {
            if body.len() < 19
                || &body[..8] != b"OpusHead"
                || body[8] != 1
                || !(1..=2).contains(&body[9])
                || body[18] != 0
            {
                return Err(OmmError::invalid(
                    "Opus requires a single mono/stereo stream",
                ));
            }
            channels = body[9] as u16;
            pre_skip = u16::from_le_bytes(body[10..12].try_into().unwrap()) as u64;
            first = false;
        }
        let position = u64::from_le_bytes(header[6..14].try_into().unwrap());
        if position != u64::MAX {
            granule = granule.max(position);
        }
    }
    let duration = granule.saturating_sub(pre_skip) * 1000 / 48000;
    if duration == 0 || duration > 28_800_000 {
        return Err(OmmError::invalid("Opus duration must be at most 8 hours"));
    }
    Ok((duration, channels, 48000))
}
pub async fn select(paths: &Paths, owner: &ProfileOwner, path: &Path) -> Result<Value> {
    let canonical = path.canonicalize()?;
    let input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&canonical)?;
    let metadata = input.metadata()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_BYTES {
        return Err(OmmError::invalid(
            "import requires a regular audio file no larger than 2 GiB",
        ));
    }
    let _lock = staging::required_lock(paths)?;
    staging::reap_locked(paths, owner, chrono::Utc::now().timestamp_millis())?;
    let handle = Uuid::new_v4().simple().to_string();
    staging::create(paths, owner, &handle)?;
    let dir = directory(paths, &handle)?;
    let result = select_inner(
        paths,
        owner,
        canonical,
        input,
        metadata.len(),
        &handle,
        &dir,
    )
    .await;
    if result.is_err() {
        if let Err(error) = staging::remove(paths, owner, &handle) {
            tracing::warn!(code=?error.code,"owned failed import staging retained after cleanup refusal");
        }
    }
    result
}
async fn select_inner(
    paths: &Paths,
    owner: &ProfileOwner,
    canonical: PathBuf,
    mut input: File,
    expected_size: u64,
    handle: &str,
    dir: &Path,
) -> Result<Value> {
    let source = dir.join("source");
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&source)?;
    let size = std::io::copy(
        &mut std::io::Read::by_ref(&mut input).take(MAX_BYTES + 1),
        &mut output,
    )?;
    output.sync_all()?;
    if size != expected_size || size > MAX_BYTES {
        return Err(OmmError::new(
            ErrorCode::Conflict,
            false,
            "selected file changed while copying",
        ));
    }
    drop(output);
    let mut magic = [0; 4];
    File::open(&source)?.read_exact(&mut magic)?;
    let format = if &magic == b"RIFF" {
        "wav"
    } else if &magic == b"OggS" {
        "ogg_opus"
    } else {
        return Err(OmmError::invalid(
            "select a PCM16 WAV or single-stream Ogg Opus file",
        ));
    };
    let (duration, channels, sample_rate) = if format == "wav" {
        wav(&source)?
    } else {
        opus(&source)?
    };
    let mix = dir.join("imported-mix.wav");
    if format == "wav" {
        let mut reader =
            hound::WavReader::open(&source).map_err(|_| OmmError::invalid("invalid WAV"))?;
        let spec = reader.spec();
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&mix)?;
        let mut writer = hound::WavWriter::new(
            file,
            hound::WavSpec {
                channels: 1,
                sample_rate: spec.sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .map_err(|_| OmmError::invalid("WAV output failed"))?;
        fn write_mix<T: hound::Sample>(
            reader: &mut hound::WavReader<std::io::BufReader<File>>,
            writer: &mut hound::WavWriter<File>,
            channels: u16,
            scale: impl Fn(T) -> Result<f64>,
        ) -> Result<()> {
            let mut samples = reader.samples::<T>();
            while let Some(sample) = samples.next() {
                let mut sum = scale(sample.map_err(|_| OmmError::invalid("invalid WAV sample"))?)?;
                if channels == 2 {
                    sum += scale(
                        samples
                            .next()
                            .ok_or_else(|| OmmError::invalid("truncated stereo frame"))?
                            .map_err(|_| OmmError::invalid("invalid WAV sample"))?,
                    )?;
                }
                let value = (sum / channels as f64 * 32768.0)
                    .round()
                    .clamp(-32768.0, 32767.0) as i16;
                writer
                    .write_sample(value)
                    .map_err(|_| OmmError::invalid("WAV output failed"))?;
            }
            Ok(())
        }
        if spec.sample_format == hound::SampleFormat::Float {
            write_mix::<f32>(&mut reader, &mut writer, channels, |v| {
                if v.is_finite() {
                    Ok(v as f64)
                } else {
                    Err(OmmError::invalid("nonfinite WAV sample"))
                }
            })?;
        } else {
            let scale = 2_f64.powi(spec.bits_per_sample as i32 - 1);
            write_mix::<i32>(&mut reader, &mut writer, channels, |v| Ok(v as f64 / scale))?;
        }
        writer
            .finalize()
            .map_err(|_| OmmError::invalid("WAV finalize failed"))?;
    } else {
        let codec = crate::adapters::asr::codec::AudioCodec::from_paths(paths)?;
        let result = codec.import_mix(&source, &mix, dir).await;
        // A failed decoder can leave its output with the process umask. Only normalize
        // this call's known output slot after the owned decoder has drained.
        if std::fs::symlink_metadata(&mix).is_ok() {
            controlled_file(dir, &mix)?;
            use std::os::unix::fs::MetadataExt;
            let metadata = std::fs::symlink_metadata(&mix)?;
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1 {
                return Err(OmmError::invalid("decoder output ownership changed"));
            }
            std::fs::set_permissions(&mix, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        }
        result?;
    }
    File::open(&mix)?.sync_all()?;
    let (actual, _, _) = wav(&mix)?;
    if actual.abs_diff(duration) > 25 {
        return Err(OmmError::new(
            ErrorCode::Corrupt,
            false,
            "decoded duration differs from source metadata",
        ));
    }
    let source_hash = hash_file(&source)?;
    let selection = Selection {
        handle: handle.into(),
        generation: owner.generation,
        profile_id: owner.profile_id.clone(),
        display_name: canonical
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("audio")
            .chars()
            .take(256)
            .collect(),
        byte_length: size,
        format: format.into(),
        duration_ms: actual,
        channels,
        sample_rate,
        expires_at_ms: chrono::Utc::now().timestamp_millis() + 900000,
        source_hash,
        mix_hash: hash_file(&mix)?,
        meeting_id: Uuid::new_v4(),
    };
    write_private_new(
        &dir.join("selection.json"),
        &serde_json::to_vec(&selection)?,
    )?;
    std::fs::remove_file(source)?;
    sync_dir(dir)?;
    Ok(
        json!({"selection":{"selectionHandle":handle,"displayName":selection.display_name,"byteLength":size,"format":format,"durationMs":actual,"channels":channels,"sampleRate":sample_rate,"expiresAtMs":selection.expires_at_ms}}),
    )
}
pub fn load(paths: &Paths, owner: &ProfileOwner, handle: &str) -> Result<Selection> {
    let selection = staging::selection(paths, owner, handle)?;
    if selection.generation != owner.generation
        || selection.expires_at_ms < chrono::Utc::now().timestamp_millis()
    {
        return Err(OmmError::new(
            ErrorCode::Conflict,
            false,
            "import selection expired or profile changed",
        ));
    }
    Ok(selection)
}
pub fn cancel(paths: &Paths, owner: &ProfileOwner, handle: &str) -> Result<Value> {
    let _lock = staging::required_lock(paths)?;
    staging::remove(paths, owner, handle)?;
    Ok(json!({"acknowledged":true}))
}
/// Explicit import operations and an already-compatible service may reclaim owned expired staging.
pub fn reap_expired(paths: &Paths, owner: &ProfileOwner, now_ms: i64) -> Result<Value> {
    let Some(_lock) = staging::lock(paths)? else {
        return Ok(json!({"removed":0,"retained":0}));
    };
    staging::reap_locked(paths, owner, now_ms)
}
/// File promotion precedes DB admission; an uncommitted directory is not a meeting manifest.
pub fn commit(paths: &Paths, owner: &ProfileOwner, request: &Value) -> Result<Value> {
    let payload = &request["payload"];
    let handle = payload["selectionHandle"]
        .as_str()
        .ok_or_else(|| OmmError::invalid("missing selection handle"))?;
    let metadata: MeetingMetadata = serde_json::from_value(payload["metadata"].clone())?;
    let _: chrono_tz::Tz = payload["timeZone"]
        .as_str()
        .unwrap_or("")
        .parse()
        .map_err(|_| OmmError::invalid("invalid time zone"))?;
    let start = payload["startedAtMs"]
        .as_i64()
        .ok_or_else(|| OmmError::invalid("missing import time"))?;
    let _lock = staging::required_lock(paths)?;
    let mut store = Store::open(paths)?;
    let mut body = request.clone();
    body["payload"].as_object_mut().unwrap().remove("admission");
    let key = format!("import_commit:{}", request["requestId"].as_str().unwrap());
    let hash = hash_bytes(&serde_json::to_vec(&body)?);
    if let Some(result) = store.desktop_request(&key, &hash)? {
        staging::remove(paths, owner, handle)?;
        return Ok(result);
    }
    let selection = load(paths, owner, handle)?;
    if let Some(duplicate) = store.desktop_import_duplicate(&selection.source_hash)? {
        let result =
            json!({"meeting":store.desktop_row(duplicate)?,"operationId":null,"duplicate":true});
        let result = store.desktop_atomic("import_commit", request, |_| Ok(result))?;
        staging::remove(paths, owner, handle)?;
        return Ok(result);
    }
    let dir = paths
        .data
        .join("meetings")
        .join("imports")
        .join(selection.meeting_id.to_string());
    let source = directory(paths, handle)?.join("imported-mix.wav");
    let target = promotion::promote(paths, owner, &selection, &source, &dir)?;
    let meeting = SealedMeeting {
        session_id: selection.meeting_id,
        title: metadata.title.clone(),
        directory: dir,
        segments: vec![AudioSegment {
            path: target,
            duration_ms: selection.duration_ms,
            sha256: selection.mix_hash.clone(),
            capture: None,
        }],
        sealed_at_ms: start
            .checked_add(selection.duration_ms as i64)
            .ok_or_else(|| OmmError::invalid("import time overflow"))?,
    };
    let result=store.desktop_atomic("import_commit",request,|store|{if let Some(duplicate)=store.desktop_import_duplicate(&selection.source_hash)?{return Ok(json!({"meeting":store.desktop_row(duplicate)?,"operationId":null,"duplicate":true}));}let config=crate::core::config::AppConfig::load(paths)?;let task=store.enqueue(&meeting,&config.snapshot(),false)?;store.desktop_import_metadata(meeting.session_id,&metadata,start,&selection.source_hash)?;Ok(json!({"meeting":store.desktop_row(meeting.session_id)?,"operationId":task.id,"duplicate":false}))})
?;
    staging::remove(paths, owner, handle)?;
    Ok(result)
}

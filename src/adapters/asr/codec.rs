//! Locked, local-only Ogg/Opus encoding. Source WAVs are never removed here.
use crate::core::{
    contracts::*,
    paths::{private_dir, validate_private_file, Paths},
    process::{OwnedChild, Reapers},
    store::hash_file,
};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};
use tokio::{io::AsyncReadExt, process::Command};
#[derive(Clone)]
pub struct AudioCodec {
    root: PathBuf,
    hash: String,
    reapers: Reapers,
    paths: Option<Paths>,
}
pub use crate::core::contracts::{ArchiveReceipt, OpusFile};
struct Workspace {
    path: PathBuf,
    _reader: Option<std::fs::File>,
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
impl AudioCodec {
    pub fn from_paths(p: &Paths) -> Result<Self> {
        let root = p.data.join("audio-toolchain");
        crate::core::paths::validate_private_dir(&root)?;
        let root = root.canonicalize()?;
        let manifest = root.join("manifest.json");
        crate::core::paths::controlled_file(&root, &manifest)?;
        let mut codec = Self::from_manifest(&manifest)?;
        codec.paths = Some(p.clone());
        Ok(codec)
    }
    pub(crate) fn with_reapers(mut self, reapers: Reapers) -> Self {
        self.reapers = reapers;
        self
    }
    fn reader_lock(&self) -> Result<Option<std::fs::File>> {
        self.paths
            .as_ref()
            .map(|p| crate::core::store::Store::open(p)?.audio_read_lock())
            .transpose()
    }
    pub fn from_manifest(path: &Path) -> Result<Self> {
        if !path.is_absolute() || path.canonicalize()? != path {
            return Err(OmmError::invalid("codec manifest path is not canonical"));
        }
        trusted(path)?;
        use std::os::unix::fs::MetadataExt;
        let directory = std::fs::symlink_metadata(path.parent().unwrap())?;
        if !directory.is_dir()
            || directory.uid() != unsafe { libc::geteuid() }
            || directory.mode() & 0o022 != 0
        {
            return Err(OmmError::invalid("codec directory is not trusted"));
        }
        if std::fs::metadata(path)?.len() > 16 * 1024 {
            return Err(OmmError::invalid("codec manifest exceeds limit"));
        }
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        crate::core::audio_toolchain::validate_manifest(&value)?;
        let hash = value
            .get("ffmpegSha256")
            .and_then(|v| v.as_str())
            .filter(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| OmmError::invalid("codec manifest is invalid"))?
            .to_owned();
        let codec = Self {
            root: path.parent().unwrap().to_owned(),
            hash,
            reapers: Reapers::default(),
            paths: None,
        };
        codec.verify()?;
        Ok(codec)
    }
    fn verify(&self) -> Result<()> {
        let path = self.root.join("ffmpeg");
        trusted(&path)?;
        if path.canonicalize()? != path || hash_file(&path)? != self.hash {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "locked encoder hash mismatch",
            ));
        }
        Ok(())
    }
    pub async fn drain(&self) -> Result<()> {
        self.reapers.drain().await
    }
    async fn run(&self, args: Vec<std::ffi::OsString>, hold: Arc<Workspace>) -> Result<()> {
        self.verify()?;
        let mut command = Command::new("/usr/bin/nice");
        command
            .arg("-n")
            .arg("10")
            .arg(self.root.join("ffmpeg"))
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-threads",
                "1",
                "-filter_threads",
                "1",
                "-filter_complex_threads",
                "1",
            ])
            .args(args)
            .env_clear()
            .env("OMP_NUM_THREADS", "1")
            .current_dir(&hold.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(lock) = hold._reader.as_ref() {
            crate::core::process::inherit_audio_reader(&mut command, lock);
        }
        let process = command
            .spawn()
            .map_err(|_| OmmError::unavailable("locked encoder could not start"))?;
        let mut child = OwnedChild::new(process, self.reapers.clone(), Some(hold));
        let mut stderr = child.stderr.take().unwrap();
        let operation = async {
            let read = async {
                let mut total = 0;
                let mut buf = [0u8; 4096];
                loop {
                    let n = stderr.read(&mut buf).await?;
                    if n == 0 {
                        break;
                    }
                    total += n;
                    if total > 65536 {
                        return Err(OmmError::invalid("encoder diagnostics exceeded bound"));
                    }
                }
                Ok::<(), OmmError>(())
            };
            let (status, ()) =
                tokio::try_join!(async { Ok::<_, OmmError>(child.wait().await?) }, read)?;
            if !status.success() {
                return Err(OmmError::new(
                    ErrorCode::Backend,
                    true,
                    "audio codec failed; diagnostics withheld",
                ));
            }
            Ok(())
        };
        let result = tokio::time::timeout(std::time::Duration::from_secs(1800), operation)
            .await
            .map_err(|_| OmmError::new(ErrorCode::Timeout, true, "audio codec timed out"))?;
        if result.is_err() {
            child.kill().await?;
            child.wait().await?;
        }
        result
    }
    pub async fn archive(
        &self,
        context: &TaskContext,
        audio: &AudioSegment,
        sequence: u32,
        bitrate: u32,
    ) -> Result<ArchiveReceipt> {
        if ![32000, 48000, 64000, 96000].contains(&bitrate) {
            return Err(OmmError::invalid("Opus bitrate is invalid"));
        }
        let reader_lock = self.reader_lock()?;
        validate_private_file(&audio.path)?;
        if audio.path.canonicalize()? != audio.path
            || !audio.path.starts_with(&context.meeting_dir)
            || hash_file(&audio.path)? != audio.sha256
        {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "archive source identity mismatch",
            ));
        }
        let wav = hound::WavReader::open(&audio.path)
            .map_err(|_| OmmError::invalid("archive source WAV invalid"))?;
        let frames = u64::from(wav.duration());
        let spec = wav.spec();
        if frames == 0
            || spec.channels != 2
            || spec.sample_rate != 48000
            || spec.bits_per_sample != 16
        {
            return Err(OmmError::invalid("archive source format invalid"));
        }
        let stage = context
            .meeting_dir
            .join(format!(".opus-stage-{}", uuid::Uuid::new_v4()));
        private_dir(&stage)?;
        let workspace = Arc::new(Workspace {
            path: stage,
            _reader: reader_lock,
        });
        let mut files = vec![];
        for (source, filter) in [
            ("mix", "pan=mono|c0=0.5*c0+0.5*c1"),
            ("mic", "pan=mono|c0=c0"),
            ("system", "pan=mono|c0=c1"),
        ] {
            let path = workspace.path.join(format!("{source}.opus"));
            let args = [
                "-i".into(),
                audio.path.clone().into_os_string(),
                "-af".into(),
                filter.into(),
                "-ar".into(),
                "48000".into(),
                "-ac".into(),
                "1".into(),
                "-c:a".into(),
                "libopus".into(),
                "-b:a".into(),
                bitrate.to_string().into(),
                "-vbr".into(),
                "on".into(),
                "-application".into(),
                "voip".into(),
                "-threads".into(),
                "1".into(),
                "-f".into(),
                "ogg".into(),
                "-n".into(),
                path.clone().into_os_string(),
            ]
            .to_vec();
            self.run(args, workspace.clone()).await?;
            set_private(&path)?;
            validate_opus(&path)?;
            let decoded = workspace.path.join(format!("{source}.wav"));
            self.decode_one(&path, &decoded, workspace.clone()).await?;
            let decoded_reader = hound::WavReader::open(&decoded)
                .map_err(|_| OmmError::invalid("Opus decode validation failed"))?;
            if u64::from(decoded_reader.duration()) != frames
                || decoded_reader.spec().channels != 1
                || decoded_reader.spec().sample_rate != 48000
            {
                return Err(OmmError::new(
                    ErrorCode::Corrupt,
                    false,
                    "Opus frame count mismatch including skip/padding",
                ));
            }
            std::fs::File::open(&path)?.sync_all()?;
            let (pre_skip, tail_padding) = crate::core::opus::metadata(&path, frames)?;
            files.push(OpusFile {
                pre_skip,
                tail_padding,
                decoded_frames: frames,
                sha256: hash_file(&path)?,
                path,
                source: source.into(),
                frames,
                sample_rate: 48000,
                channels: 1,
                container: "ogg".into(),
                codec: "opus".into(),
                bitrate,
            });
        }
        // Publication is a private immutable group; SQLite receipt commits its identity afterwards.
        let dir = context
            .meeting_dir
            .join(format!("archive-{sequence:04}-{}", context.lease.token));
        if std::fs::symlink_metadata(&dir).is_ok() {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "archive destination already exists",
            ));
        }
        let sources = workspace.path.join(".sources");
        private_dir(&sources)?;
        for file in &mut files {
            std::fs::remove_file(workspace.path.join(format!("{}.wav", file.source)))?;
            let relative = if file.source == "mix" {
                PathBuf::from("mix.opus")
            } else {
                PathBuf::from(".sources").join(format!("{}.opus", file.source))
            };
            let stage_file = workspace.path.join(&relative);
            if file.path != stage_file {
                std::fs::rename(&file.path, &stage_file)?;
            }
            file.path = dir.join(relative);
        }
        std::fs::File::open(&sources)?.sync_all()?;
        std::fs::File::open(&workspace.path)?.sync_all()?;
        std::fs::rename(&workspace.path, &dir)?;
        std::fs::File::open(&context.meeting_dir)?.sync_all()?;
        self.drain().await?;
        Ok(ArchiveReceipt {
            encoder_version: Some(format!("ffmpeg-8.1.2/libopus-1.5.2/sha256:{}", self.hash)),
            schema_version: 1,
            source_sha256: audio.sha256.clone(),
            files,
        })
    }
    async fn decode_one(&self, input: &Path, output: &Path, hold: Arc<Workspace>) -> Result<()> {
        validate_opus(input)?;
        self.run(
            vec![
                "-i".into(),
                input.as_os_str().to_owned(),
                "-ar".into(),
                "48000".into(),
                "-ac".into(),
                "1".into(),
                "-c:a".into(),
                "pcm_s16le".into(),
                "-threads".into(),
                "1".into(),
                "-f".into(),
                "wav".into(),
                "-n".into(),
                output.as_os_str().to_owned(),
            ],
            hold,
        )
        .await?;
        set_private(output)
    }
    pub async fn decode_sources(
        &self,
        receipt: &ArchiveReceipt,
        context: &TaskContext,
    ) -> Result<AudioSegment> {
        // An ASR attempt may be cancelled while the decoder child is still being reaped.
        // Keep its child-owned scratch outside the caller's disposable attempt directory.
        let work = context
            .meeting_dir
            .join(format!(".decode-source-{}", uuid::Uuid::new_v4()));
        private_dir(&work)?;
        let hold = Arc::new(Workspace {
            path: work,
            _reader: self.reader_lock()?,
        });
        let mut readers = vec![];
        let mut frames = None;
        for source in ["mic", "system"] {
            let file = receipt
                .files
                .iter()
                .find(|f| f.source == source)
                .ok_or_else(|| OmmError::invalid("source Opus missing"))?;
            validate_private_file(&file.path)?;
            if !file.path.starts_with(&context.meeting_dir) || hash_file(&file.path)? != file.sha256
            {
                return Err(OmmError::new(
                    ErrorCode::Corrupt,
                    false,
                    "source Opus identity mismatch",
                ));
            }
            if frames.is_some_and(|n| n != file.frames) {
                return Err(OmmError::invalid("source frame mismatch"));
            }
            frames = Some(file.frames);
            let output = hold.path.join(format!("{source}.wav"));
            self.decode_one(&file.path, &output, hold.clone()).await?;
            let reader = hound::WavReader::open(output)
                .map_err(|_| OmmError::invalid("decoded source invalid"))?;
            if u64::from(reader.duration()) != file.frames {
                return Err(OmmError::invalid("decoded source frame mismatch"));
            }
            readers.push(reader);
        }
        let output = context
            .work_dir
            .join(format!("decoded-{}.wav", uuid::Uuid::new_v4()));
        let mut writer = hound::WavWriter::create(
            &output,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .map_err(|_| OmmError::unavailable("decode output unavailable"))?;
        let (mut a, mut b) = (readers.remove(0), readers.remove(0));
        let (mut a, mut b) = (a.samples::<i16>(), b.samples::<i16>());
        for _ in 0..frames.unwrap_or(0) {
            writer
                .write_sample(
                    a.next()
                        .ok_or_else(|| OmmError::invalid("mic truncated"))?
                        .map_err(|_| OmmError::invalid("mic sample invalid"))?,
                )
                .map_err(|_| OmmError::unavailable("decode write failed"))?;
            writer
                .write_sample(
                    b.next()
                        .ok_or_else(|| OmmError::invalid("system truncated"))?
                        .map_err(|_| OmmError::invalid("system sample invalid"))?,
                )
                .map_err(|_| OmmError::unavailable("decode write failed"))?;
        }
        writer
            .finalize()
            .map_err(|_| OmmError::unavailable("decode finalize failed"))?;
        set_private(&output)?;
        self.drain().await?;
        Ok(AudioSegment { capture: None,
            sha256: hash_file(&output)?,
            path: output,
            duration_ms: frames.unwrap_or(0) * 1000 / 48000,
        })
    }
}
fn set_private(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}
fn trusted(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(path)?;
    if !m.is_file() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o022 != 0 {
        return Err(OmmError::invalid(
            "codec runtime is not a trusted owned file",
        ));
    }
    Ok(())
}
fn validate_opus(path: &Path) -> Result<()> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut header = [0u8; 47];
    f.read_exact(&mut header)?;
    if &header[..4] != b"OggS"
        || &header[28..36] != b"OpusHead"
        || header[37] != 1
        || u32::from_le_bytes(header[40..44].try_into().unwrap()) != 48000
    {
        return Err(OmmError::new(
            ErrorCode::Corrupt,
            false,
            "audio is not mono Ogg/Opus at 48kHz",
        ));
    }
    Ok(())
}

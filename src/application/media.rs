//! Verified public-artifact reads. The descriptor and shared maintenance lock outlive each read.
use crate::core::{contracts::*, store::hash_bytes};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    },
    path::{Component, Path},
};

pub const MAX_RESPONSE: u64 = 8 * 1024 * 1024;
#[derive(PartialEq, Eq)]
struct ArtifactStamp {
    device: u64,
    inode: u64,
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl ArtifactStamp {
    fn read(file: &File) -> std::io::Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            length: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}
pub struct VerifiedMedia {
    file: File,
    stamp: ArtifactStamp,
    _reader_lock: File,
    pub length: u64,
    pub hash: String,
    pub mime: &'static str,
}
pub struct MediaResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
fn unsafe_file() -> OmmError {
    OmmError::new(ErrorCode::Permission, false, "unsafe media artifact")
}
fn open_child(parent: &File, name: &std::ffi::OsStr, directory: bool) -> Result<File> {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| unsafe_file())?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | if directory { libc::O_DIRECTORY } else { 0 },
        )
    };
    if fd < 0 {
        return Err(unsafe_file());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
impl VerifiedMedia {
    pub fn open(
        root: &Path,
        relative: &Path,
        expected_hash: &str,
        reader_lock: File,
        mime: &'static str,
    ) -> Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        if root.canonicalize()? != root || relative.is_absolute() || expected_hash.len() != 64 {
            return Err(unsafe_file());
        }
        let components: Vec<_> = relative.components().collect();
        if components.is_empty()
            || components
                .iter()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(unsafe_file());
        }
        let mut parent = File::options()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(root)?;
        for (i, component) in components.iter().enumerate() {
            let meta = parent.metadata()?;
            if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
                return Err(unsafe_file());
            }
            parent = open_child(&parent, component.as_os_str(), i + 1 < components.len())?;
        }
        let meta = parent.metadata()?;
        if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err(unsafe_file());
        }
        let stamp = ArtifactStamp::read(&parent)?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let count = parent.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        let actual = hex::encode(hash.finalize());
        if actual != expected_hash || ArtifactStamp::read(&parent)? != stamp {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "media artifact hash mismatch",
            ));
        }
        parent.seek(SeekFrom::Start(0))?;
        Ok(Self {
            file: parent,
            stamp,
            _reader_lock: reader_lock,
            length: meta.len(),
            hash: actual,
            mime,
        })
    }
    pub fn response(
        &mut self,
        method: &str,
        range: Option<&str>,
        if_range: Option<&str>,
    ) -> Result<MediaResponse> {
        let stamp = ArtifactStamp::read(&self.file)?;
        if stamp != self.stamp {
            let metadata = self.file.metadata()?;
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
                return Err(unsafe_file());
            }
            self.file.seek(SeekFrom::Start(0))?;
            let mut hash = Sha256::new();
            let mut buffer = [0; 65536];
            loop {
                let count = self.file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
            if hex::encode(hash.finalize()) != self.hash
                || ArtifactStamp::read(&self.file)? != stamp
            {
                return Err(OmmError::new(
                    ErrorCode::Corrupt,
                    false,
                    "media artifact changed after verification",
                ));
            }
            // A rename can change ctime without altering the verified descriptor's bytes.
            self.stamp = stamp;
        }
        let etag = format!("\"{}\"", self.hash);
        let mut headers = vec![
            ("Accept-Ranges".into(), "bytes".into()),
            ("ETag".into(), etag.clone()),
            ("Content-Type".into(), self.mime.into()),
            ("Cache-Control".into(), "no-store".into()),
            ("X-Content-Type-Options".into(), "nosniff".into()),
        ];
        let mut error = |status, value: Option<String>| {
            if let Some(value) = value {
                headers.push(("Content-Range".into(), value));
            }
            MediaResponse {
                status,
                headers: headers.clone(),
                body: vec![],
            }
        };
        if method != "GET" && method != "HEAD" {
            return Ok(error(405, None));
        }
        if if_range.is_some_and(|value| value != etag) {
            return Ok(error(412, None));
        }
        let interval = range
            .map(|value| parse_range(value, self.length))
            .transpose();
        let interval = match interval {
            Ok(value) => value,
            Err(_) => return Ok(error(416, Some(format!("bytes */{}", self.length)))),
        };
        let (start, end, status) = match interval {
            Some((start, end)) => (start, end, 206),
            None if self.length > 0 => (0, self.length - 1, 200),
            None => return Ok(error(416, Some("bytes */0".into()))),
        };
        let length = end - start + 1;
        if method == "GET" && length > MAX_RESPONSE {
            return Ok(error(413, None));
        }
        headers.push(("Content-Length".into(), length.to_string()));
        if status == 206 {
            headers.push((
                "Content-Range".into(),
                format!("bytes {start}-{end}/{}", self.length),
            ));
        }
        let mut body = vec![];
        if method == "GET" {
            self.file.seek(SeekFrom::Start(start))?;
            body.resize(length as usize, 0);
            self.file.read_exact(&mut body)?;
            if ArtifactStamp::read(&self.file)? != self.stamp {
                return Err(OmmError::new(
                    ErrorCode::Corrupt,
                    false,
                    "media artifact changed during read",
                ));
            }
        }
        Ok(MediaResponse {
            status,
            headers,
            body,
        })
    }
}
pub fn parse_range(value: &str, length: u64) -> Result<(u64, u64)> {
    let invalid = || OmmError::invalid("invalid media range");
    let value = value.strip_prefix("bytes=").ok_or_else(invalid)?;
    if value.contains(',') || length == 0 {
        return Err(invalid());
    }
    let (start, end) = value.split_once('-').ok_or_else(invalid)?;
    let integer = |text: &str| -> Result<u64> {
        if text.is_empty() || !text.bytes().all(|c| c.is_ascii_digit()) {
            return Err(invalid());
        }
        text.parse().map_err(|_| invalid())
    };
    if start.is_empty() {
        let suffix = integer(end)?;
        if suffix == 0 {
            return Err(invalid());
        }
        return Ok((length.saturating_sub(suffix), length - 1));
    }
    let start = integer(start)?;
    let end = if end.is_empty() {
        length - 1
    } else {
        integer(end)?.min(length - 1)
    };
    if start >= length || end < start {
        return Err(invalid());
    }
    Ok((start, end))
}
/// Synthetic silence used only by the explicitly compiled WKWebView fixture binary.
pub fn silent_wav(seconds: u32) -> Vec<u8> {
    let data = seconds * 16000 * 2;
    let mut bytes = Vec::with_capacity(data as usize + 44);
    bytes.extend(b"RIFF");
    bytes.extend((data + 36).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(16000u32.to_le_bytes());
    bytes.extend(32000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(data.to_le_bytes());
    bytes.resize(data as usize + 44, 0);
    bytes
}
pub fn silence_hash(seconds: u32) -> String {
    hash_bytes(&silent_wav(seconds))
}

pub struct MediaTarget {
    pub segment_id: uuid::Uuid,
    pub path: std::path::PathBuf,
    pub hash: String,
    pub mime: &'static str,
    pub duration: u64,
    pub offset: u64,
    pub sequence: usize,
}
pub fn targets(store: &crate::core::store::Store, meeting: uuid::Uuid) -> Result<Vec<MediaTarget>> {
    let meeting = store
        .meeting(meeting)?
        .ok_or_else(|| OmmError::unavailable("meeting not found"))?;
    let mut targets = Vec::new();
    let mut offset = 0;
    for (sequence, audio) in meeting.segments.iter().enumerate() {
        let archived = store
            .archive_for_audio(meeting.session_id, audio)?
            .and_then(|r| r.files.into_iter().find(|f| f.source == "mix"));
        let (path, hash, mime) = if let Some(file) = archived {
            (file.path, file.sha256, "audio/ogg")
        } else {
            (audio.path.clone(), audio.sha256.clone(), "audio/wav")
        };
        let identity = hash_bytes(format!("{}:{sequence}:{hash}", meeting.session_id).as_bytes());
        let segment_id = uuid::Uuid::from_slice(
            &hex::decode(&identity[..32]).map_err(|_| OmmError::invalid("media identity"))?,
        )
        .map_err(|_| OmmError::invalid("media identity"))?;
        targets.push(MediaTarget {
            segment_id,
            path,
            hash,
            mime,
            duration: audio.duration_ms,
            offset,
            sequence,
        });
        offset += audio.duration_ms;
    }
    Ok(targets)
}
pub fn manifest(
    store: &crate::core::store::Store,
    paths: &crate::core::paths::Paths,
    payload: &serde_json::Value,
) -> Result<serde_json::Value> {
    use serde_json::json;
    let meeting = super::id(payload, "meetingId")?;
    let targets = targets(store, meeting)?;
    let mut rows = Vec::new();
    for target in &targets {
        let verified = target
            .path
            .strip_prefix(&paths.data)
            .ok()
            .and_then(|relative| {
                VerifiedMedia::open(
                    &paths.data,
                    relative,
                    &target.hash,
                    store.audio_read_lock().ok()?,
                    target.mime,
                )
                .ok()
            });
        rows.push(json!({"segmentId":target.segment_id,"sequence":target.sequence,"offsetMs":target.offset,"durationMs":target.duration,"mime":target.mime,"byteLength":verified.as_ref().map(|v|v.length).unwrap_or(1),"sha256":target.hash,"available":verified.is_some(),"reason":if verified.is_some(){None}else{Some("not_found")}}));
    }
    let page = super::page(rows, payload)?;
    Ok(
        json!({"meetingId":meeting,"revision":page["revision"],"segments":page["items"],"nextCursor":page["nextCursor"],"durationMs":targets.iter().map(|t|t.duration).sum::<u64>(),"complete":page["nextCursor"].is_null()}),
    )
}

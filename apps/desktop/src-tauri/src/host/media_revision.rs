//! Manifest revisions authorize a snapshot; artifact hashes independently verify audio bytes.
use super::*;
use omm::core::{config::AppConfig, paths::*, store::hash_bytes};
use std::{collections::BTreeMap, path::PathBuf};

struct Fixture {
    root: PathBuf,
    host: Host,
    store: Store,
    meeting: SealedMeeting,
}
impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from("/private/tmp")
            .join(format!("omm-media-revision-test-{}", Uuid::new_v4()));
        private_dir(&root).unwrap();
        let paths = Paths::resolve(&root.join("home"), &BTreeMap::new()).unwrap();
        paths.create_private_dirs().unwrap();
        let mut config = AppConfig::default();
        config.notifications.enabled = false;
        let id = Uuid::new_v4();
        let directory = paths.data.join("meetings").join(id.to_string());
        private_dir(&directory).unwrap();
        let segments = (1..=2)
            .map(|seconds| {
                let wav = app::media::silent_wav(seconds);
                let path = directory.join(format!("segment-{seconds}.wav"));
                write_private_new(&path, &wav).unwrap();
                AudioSegment {
                    capture: None,
                    path,
                    duration_ms: u64::from(seconds) * 1000,
                    sha256: hash_bytes(&wav),
                }
            })
            .collect();
        let meeting = SealedMeeting {
            session_id: id,
            title: "private synthetic media revision".into(),
            directory,
            sealed_at_ms: 1700000000000,
            segments,
        };
        let mut store = Store::open(&paths).unwrap();
        store.enqueue(&meeting, &config.snapshot(), false).unwrap();
        Self {
            root,
            host: Host::new(paths),
            store,
            meeting,
        }
    }
    async fn call(&self, command: &str, payload: Value) -> Result<Value> {
        self.host
            .dispatch(
                command,
                &json!({"schemaVersion":1,"requestId":Uuid::new_v4().to_string(),"payload":payload}),
            )
            .await
    }
    async fn manifest(&self, page_size: u64) -> Value {
        self.call(
            "media_manifest",
            json!({"meetingId":self.meeting.session_id,"cursor":null,"pageSize":page_size}),
        )
        .await
        .unwrap()
    }
    async fn open(&self, segment: &Value, revision: &Value) -> Result<Value> {
        self.call(
            "media_open",
            json!({"meetingId":self.meeting.session_id,"segmentId":segment,"expectedRevision":revision}),
        )
        .await
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.host.grants.lock().unwrap().clear();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn media_open_returns_manifest_revision_and_keeps_artifact_etag_and_reader_lifetime() {
    tauri::async_runtime::block_on(async {
        let f = Fixture::new();
        let manifest = f.manifest(1).await;
        assert!(!manifest["nextCursor"].is_null());
        assert_eq!(manifest["revision"], f.manifest(100).await["revision"]);
        assert!(!f.store.active_audio_readers().unwrap());
        let segment = &manifest["segments"][0];
        assert_ne!(manifest["revision"], segment["sha256"]);
        let wrong_revision = f
            .open(&segment["segmentId"], &json!("0".repeat(64)))
            .await
            .unwrap_err();
        assert_eq!(wrong_revision.code, ErrorCode::Conflict);
        assert!(f.host.grants.lock().unwrap().is_empty());
        assert!(!f.store.active_audio_readers().unwrap());

        let grant = f
            .open(&segment["segmentId"], &manifest["revision"])
            .await
            .unwrap();
        assert_eq!(grant["revision"], manifest["revision"]);
        assert_eq!(
            grant["url"],
            format!(
                "omm-media://localhost/{}",
                grant["grantId"].as_str().unwrap()
            )
        );
        assert!(f.store.active_audio_readers().unwrap());
        {
            let mut grants = f.host.grants.lock().unwrap();
            let held = grants.get_mut(grant["grantId"].as_str().unwrap()).unwrap();
            assert_eq!(held.media.hash, segment["sha256"].as_str().unwrap());
            let artifact_etag = format!("\"{}\"", held.media.hash);
            let response = held
                .media
                .response("GET", Some("bytes=0-43"), Some(&artifact_etag))
                .unwrap();
            assert_eq!(response.status, 206);
            assert_eq!(response.body.len(), 44);
            assert!(response.headers.contains(&("ETag".into(), artifact_etag)));
            let manifest_etag = format!("\"{}\"", manifest["revision"].as_str().unwrap());
            assert_eq!(
                held.media
                    .response("GET", Some("bytes=0-43"), Some(&manifest_etag))
                    .unwrap()
                    .status,
                412
            );
        }
        for _ in 0..2 {
            assert_eq!(
                f.call("media_close", json!({"grantId":grant["grantId"]}))
                    .await
                    .unwrap()["acknowledged"],
                true
            );
        }
        assert!(f.host.grants.lock().unwrap().is_empty());
        assert!(!f.store.active_audio_readers().unwrap());
    });
}

#[test]
fn media_open_rejects_stale_manifest_and_independently_rejects_corrupt_audio() {
    tauri::async_runtime::block_on(async {
        let f = Fixture::new();
        let manifest = f.manifest(100).await;
        let segment = &manifest["segments"][0];
        let mut wav = std::fs::read(&f.meeting.segments[0].path).unwrap();
        wav[44] ^= 1;
        std::fs::write(&f.meeting.segments[0].path, &wav).unwrap();
        let stale = f
            .open(&segment["segmentId"], &manifest["revision"])
            .await
            .unwrap_err();
        assert_eq!(stale.code, ErrorCode::Conflict);
        let current = f.manifest(100).await;
        assert_ne!(current["revision"], manifest["revision"]);
        assert_eq!(current["segments"][0]["available"], false);
        // A current manifest token never substitutes for the stored artifact hash.
        let corrupt = f
            .open(&segment["segmentId"], &current["revision"])
            .await
            .unwrap_err();
        assert_eq!(corrupt.code, ErrorCode::Corrupt);
        assert!(f.host.grants.lock().unwrap().is_empty());
        assert!(!f.store.active_audio_readers().unwrap());
    });
}

use async_trait::async_trait;
use omm::core::{
    contracts::*,
    models::*,
    paths::*,
    store::{hash_bytes, Store},
};
use std::{collections::VecDeque, fs};
fn setup() -> (tempfile::TempDir, Paths) {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let p = Paths::resolve(&root.join("home"), &Default::default()).unwrap();
    p.create_private_dirs().unwrap();
    (t, p)
}
fn asset(bytes: &[u8]) -> Asset {
    Asset {
        id: "fixture".into(),
        kind: "file".into(),
        url: "https://fixture.invalid/public-model".into(),
        sha256: hash_bytes(bytes),
        archive_bytes: bytes.len() as u64,
        installed_bytes: bytes.len() as u64,
        files: std::collections::BTreeMap::from([("model.bin".into(), hash_bytes(bytes))]),
    }
}
struct Chunks(VecDeque<Vec<u8>>);
#[async_trait]
impl DownloadSource for Chunks {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        Ok(self.0.pop_front())
    }
}
#[tokio::test]
async fn explicit_download_stream_is_pinned_private_and_cleans_failed_staging() {
    let (_t, p) = setup();
    let bytes = b"tiny synthetic model";
    let a = asset(bytes);
    let mut chunks = Chunks(VecDeque::from([bytes[..5].to_vec(), bytes[5..].to_vec()]));
    download_from_source(&p, &a, &mut chunks).await.unwrap();
    assert!(verify(&p, &a).is_ok());
    assert_eq!(
        fs::read(p.data.join("models/fixture/model.bin")).unwrap(),
        bytes
    );
    let original = fs::read(p.data.join("models/fixture/model.bin")).unwrap();
    let mut bad = Chunks(VecDeque::from([b"different model bytes".to_vec()]));
    assert!(download_from_source(&p, &a, &mut bad).await.is_err());
    assert_eq!(
        fs::read(p.data.join("models/fixture/model.bin")).unwrap(),
        original
    );
    assert_eq!(fs::read_dir(p.cache.join("downloads")).unwrap().count(), 0);
    remove(&p, &a).unwrap();
    assert!(verify(&p, &a).is_err());
}
#[test]
fn archives_reject_missing_files_and_model_mutation_lock_conflicts_with_running_asr() {
    let (t, p) = setup();
    let bytes = b"fixture";
    let mut a = asset(bytes);
    a.kind = "tarbz2".into();
    let tarpath = t.path().join("archive");
    let out = fs::File::create(&tarpath).unwrap();
    let encoder = bzip2::write::BzEncoder::new(out, bzip2::Compression::fast());
    let mut tar = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(7);
    header.set_mode(0o600);
    header.set_cksum();
    tar.append_data(&mut header, "bundle/unlisted.bin", &bytes[..])
        .unwrap();
    let enc = tar.into_inner().unwrap();
    enc.finish().unwrap();
    a.archive_bytes = fs::metadata(&tarpath).unwrap().len();
    a.sha256 = omm::core::store::hash_file(&tarpath).unwrap();
    assert_eq!(
        install_archive(&p, &a, &tarpath).unwrap_err().code,
        ErrorCode::Corrupt
    );
    let store = Store::open(&p).unwrap();
    let lock = store.model_read_lock().unwrap();
    assert_eq!(mutation_lock(&p).unwrap_err().code, ErrorCode::Conflict);
    drop(lock);
    let _mutation = mutation_lock(&p).unwrap();
    assert_eq!(
        store.model_read_lock().unwrap_err().code,
        ErrorCode::Conflict
    );
}

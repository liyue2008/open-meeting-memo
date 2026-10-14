use async_trait::async_trait;
use omm::{
    adapters::{meeting::classify, pi::PiAdapter},
    core::{
        contracts::*,
        paths::{private_dir, Paths},
        pipeline::run_one,
        store::{hash_bytes, Store},
    },
};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};
use tempfile::TempDir;

struct FixtureAsr;
#[async_trait]
impl AsrAdapter for FixtureAsr {
    async fn transcribe(&self, req: AsrRequest) -> Result<Transcript> {
        assert_eq!(req.segments.len(), 1);
        Ok(Transcript {
            schema_version: 1,
            model_version: "fixture-asr".into(),
            duration_ms: 1000,
            segments: vec![TranscriptSegment {
                source: AudioSource::Mic,
                start_ms: 0,
                end_ms: 900,
                text: "TRANSCRIPT_FIXTURE_NOT_ARGV".into(),
            }],
        })
    }
}
fn setup(script: &str) -> (TempDir, Paths, Store, Task) {
    let t = TempDir::new().unwrap();
    let root = t.path().canonicalize().unwrap();
    let paths = Paths::resolve(&root.join("home"), &BTreeMap::new()).unwrap();
    paths.create_private_dirs().unwrap();
    let executable = root.join("fake-pi");
    fs::write(&executable, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let session_id = uuid::Uuid::new_v4();
    let dir = paths.data.join("meetings").join(session_id.to_string());
    private_dir(&dir).unwrap();
    let audio = dir.join("segment.wav");
    fs::write(&audio, b"synthetic").unwrap();
    let meeting = SealedMeeting {
        session_id,
        title: "synthetic".into(),
        directory: dir,
        sealed_at_ms: 1,
        segments: vec![AudioSegment {
            capture: None,
            path: audio,
            duration_ms: 1000,
            sha256: hash_bytes(b"synthetic"),
        }],
    };
    let cfg = ConfigSnapshot {
        audio: Default::default(),
        asr: AsrConfig {
            model_version: "fixture-asr".into(),
            ..AsrConfig::default()
        },
        pi: PiConfig {
            executable,
            provider: "fixture-provider".into(),
            model: "fixture-model".into(),
            prompt_version: "1".into(),
            timeout_ms: 5000,
            max_input_bytes: 10000,
            max_output_bytes: 10000,
        },
        ..ConfigSnapshot::default()
    };
    let mut store = Store::open(&paths).unwrap();
    let task = store.enqueue(&meeting, &cfg, false).unwrap();
    (t, paths, store, task)
}
#[tokio::test]
async fn transcript_commit_automatically_runs_print_pi_with_stdin_and_no_tools_or_discovery() {
    let script = r#"
for flag in --print --no-session --no-tools --no-mcp --no-extensions --no-skills --no-prompt-templates --no-context-files --no-approve --offline; do
  case " $* " in *" $flag "*) ;; *) exit 70;; esac
done
case "$*" in *TRANSCRIPT_FIXTURE_NOT_ARGV*) exit 71;; esac
[ -z "$PI_SESSION_FILE" ] || exit 72
[ -z "$OPENAI_API_KEY" ] || exit 73
case "$PI_CODING_AGENT_DIR" in */omm/pi) ;; *) exit 74;; esac
case "$HOME" in */omm/pi/*/home) ;; *) exit 75;; esac
input=$(/bin/cat)
case "$input" in *TRANSCRIPT_FIXTURE_NOT_ARGV*) ;; *) exit 76;; esac
case "$input" in *segment.wav*|*state.sqlite*) exit 77;; esac
printf '# Fixture minutes\nDecision: test passed.\n'
"#;
    let (_t, paths, mut store, task) = setup(script);
    let pi = PiAdapter::new(paths.clone());
    let transcribed = run_one(&mut store, &FixtureAsr, &pi, 10000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(transcribed.status, TaskStatus::TranscriptCommitted);
    let result = run_one(&mut store, &FixtureAsr, &pi, 10000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.status, TaskStatus::Completed, "{:?}", result.error);
    assert!(
        String::from_utf8(store.read_artifact(task.id, Stage::Summary).unwrap())
            .unwrap()
            .contains("Fixture minutes")
    );
    assert!(run_one(&mut store, &FixtureAsr, &pi, 10000)
        .await
        .unwrap()
        .is_none());
    assert_eq!(fs::read_dir(paths.cache.join("pi")).unwrap().count(), 0);
}
#[tokio::test]
async fn pi_failure_does_not_disclose_stderr_and_only_summary_retries() {
    let (_t, paths, mut store, task) = setup("echo 'SECRET_FIXTURE_DO_NOT_DISCLOSE' >&2; exit 1");
    let pi = PiAdapter::new(paths);
    run_one(&mut store, &FixtureAsr, &pi, 10000).await.unwrap();
    let before = store.read_artifact(task.id, Stage::Transcribe).unwrap();
    let failed = run_one(&mut store, &FixtureAsr, &pi, 10000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, TaskStatus::Failed);
    assert!(!serde_json::to_string(&failed)
        .unwrap()
        .contains("SECRET_FIXTURE"));
    store.retry(task.id).unwrap();
    let claim = store
        .claim(chrono::Utc::now().timestamp_millis(), 10000)
        .unwrap()
        .unwrap();
    assert_eq!(claim.stage, Stage::Summary);
    assert_eq!(
        before,
        store.read_artifact(task.id, Stage::Transcribe).unwrap()
    );
}
#[tokio::test]
async fn pi_timeout_and_output_limit_are_errors_not_success() {
    let (_t, paths, mut store, _) = setup("sleep 6; printf 'late'");
    let pi = PiAdapter::new(paths);
    run_one(&mut store, &FixtureAsr, &pi, 10000).await.unwrap();
    let timeout = run_one(&mut store, &FixtureAsr, &pi, 10000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(timeout.error.unwrap().code, ErrorCode::Timeout);
    let (_t, paths, mut store, _) = setup("/usr/bin/head -c 10001 /dev/zero");
    let pi = PiAdapter::new(paths);
    run_one(&mut store, &FixtureAsr, &pi, 10000).await.unwrap();
    let overflow = run_one(&mut store, &FixtureAsr, &pi, 10000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(overflow.status, TaskStatus::Failed);
}
#[test]
fn lark_strong_signal_requires_joined_participant_and_fresh_matching_room() {
    let strong = classify(1, Some(true), None, Some(2), Some("room"), Some("room"));
    assert_eq!(strong.evidence, Evidence::Strong);
    assert_eq!(
        classify(1, Some(true), None, Some(2), Some("room"), Some("other")).evidence,
        Evidence::Weak
    );
    assert_eq!(
        classify(1, None, None, None, None, None).state,
        MeetingState::Unknown
    );
    assert_eq!(
        classify(1, Some(false), None, None, None, None).state,
        MeetingState::Inactive
    );
}

#[tokio::test]
async fn large_transcript_is_chunked_with_fixed_stdin_and_authentication_failure_blocks_only_summary(
) {
    let (t,paths,mut store,_)=setup("input=$(/bin/cat); printf x >> \"$(/usr/bin/dirname \"$0\")/pi-count\"; printf '# chunk minutes\\n'");
    let claim = store
        .claim(chrono::Utc::now().timestamp_millis(), 10000)
        .unwrap()
        .unwrap();
    let mut context = claim.context;
    context.config.pi.max_input_bytes = 2000;
    let transcript = Transcript {
        schema_version: 1,
        model_version: "fixture-asr".into(),
        duration_ms: 1000,
        segments: (0..10)
            .map(|n| TranscriptSegment {
                source: AudioSource::Mic,
                start_ms: n * 90,
                end_ms: n * 90 + 80,
                text: "SYNTHETIC_NO_AUDIO ".repeat(12),
            })
            .collect(),
    };
    let pi = PiAdapter::new(paths);
    assert!(pi
        .summarize(SummaryRequest {
            context,
            transcript
        })
        .await
        .unwrap()
        .markdown
        .contains("chunk minutes"));
    assert!(fs::read(t.path().join("pi-count")).unwrap().len() > 1);
    let (_t, p, mut s, task) =
        setup("/bin/cat >/dev/null; printf 'Unauthorized SECRET_SYNTHETIC_KEY' >&2; exit 1");
    let pi = PiAdapter::new(p);
    run_one(&mut s, &FixtureAsr, &pi, 10000).await.unwrap();
    let blocked = run_one(&mut s, &FixtureAsr, &pi, 10000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(blocked.status, TaskStatus::Blocked);
    assert_eq!(blocked.stage, Stage::Summary);
    assert!(!serde_json::to_string(&blocked)
        .unwrap()
        .contains("SECRET_SYNTHETIC"));
    assert!(s.read_artifact(task.id, Stage::Transcribe).is_ok());
}
struct SlowAsr;
#[async_trait]
impl AsrAdapter for SlowAsr {
    async fn transcribe(&self, req: AsrRequest) -> Result<Transcript> {
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        FixtureAsr.transcribe(req).await
    }
}
#[tokio::test]
async fn pipeline_renews_lease_during_backend_wait_and_cleans_disposable_work() {
    let (_t, paths, mut store, task) = setup("/bin/cat >/dev/null; printf '# minutes'");
    let pi = PiAdapter::new(paths.clone());
    let operation = run_one(&mut store, &SlowAsr, &pi, 250);
    let probe = async {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let mut other = Store::open(&paths).unwrap();
        assert!(other
            .claim(chrono::Utc::now().timestamp_millis(), 250)
            .unwrap()
            .is_none());
    };
    let (processed, ()) = tokio::join!(operation, probe);
    assert_eq!(
        processed.unwrap().unwrap().status,
        TaskStatus::TranscriptCommitted
    );
    assert!(store.read_artifact(task.id, Stage::Transcribe).is_ok());
    assert!(!paths.cache.join("tasks").join(task.id.to_string()).exists());
}

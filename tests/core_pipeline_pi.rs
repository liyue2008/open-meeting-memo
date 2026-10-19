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
            node_executable: None,
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
    let directories = fs::read_dir(paths.cache.join("pi"))
        .unwrap()
        .collect::<std::io::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        directories.len(),
        1,
        "owned Pi wait is not full background coverage; retain its private workspace"
    );
    let marker = directories[0].path().join("owner.json");
    let owner: serde_json::Value = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
    assert_eq!(owner["background_unverified"], true);
    assert_eq!(
        omm::adapters::pi::recover_workspaces(&paths, &store)
            .unwrap()
            .removed,
        0
    );
    assert!(marker.exists());
    let mut legacy = owner;
    legacy
        .as_object_mut()
        .unwrap()
        .remove("background_unverified");
    omm::core::paths::write_private_replace(&marker, &serde_json::to_vec(&legacy).unwrap())
        .unwrap();
    assert_eq!(
        omm::adapters::pi::recover_workspaces(&paths, &store)
            .unwrap()
            .retained_unknown,
        1
    );
    assert!(
        marker.exists(),
        "legacy records must not infer full-chain exit from PIDs"
    );
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

#[tokio::test]
async fn managed_stable_launcher_uses_only_explicit_owned_node_and_isolated_pi_environment() {
    let (t, paths, mut store, _) = setup("/bin/cat >/dev/null; printf '# fixture'");
    let claim = store
        .claim(chrono::Utc::now().timestamp_millis(), 10000)
        .unwrap()
        .unwrap();
    let mut context = claim.context.clone();
    let bin = t.path().join("managed/bin");
    let install = t.path().join("managed/install/releases/fixture");
    omm::core::paths::private_dir(&bin).unwrap();
    omm::core::paths::private_dir(&install).unwrap();
    fs::write(
        t.path().join("managed/install/current-version"),
        "fixture\n",
    )
    .unwrap();
    let launcher = bin.join("pi");
    fs::write(&launcher,"#!/bin/sh\nroot=$(/usr/bin/dirname \"$0\")/..\nv=$(/bin/cat \"$root/install/current-version\")\nexec \"$root/install/releases/$v/pi.js\" \"$@\"\n").unwrap();
    let js = install.join("pi.js");
    fs::write(&js,"#!/usr/bin/env node\n[ -n \"$PI_CODING_AGENT_DIR\" ] || exit 81\n[ \"$HOME\" != \"$PI_CODING_AGENT_DIR\" ] || exit 82\nfor flag in --no-tools --no-mcp --no-extensions --no-skills --no-session --no-context-files; do case \" $* \" in *\" $flag \"*) ;; *) exit 83;; esac; done\n/bin/cat >/dev/null\nprintf '# managed stable minutes'\n").unwrap();
    let runtime = t.path().join("runtime");
    omm::core::paths::private_dir(&runtime).unwrap();
    let node = runtime.join("node");
    fs::write(&node, "#!/bin/sh\nexec /bin/sh \"$@\"\n").unwrap();
    for p in [&launcher, &js, &node] {
        fs::set_permissions(p, fs::Permissions::from_mode(0o700)).unwrap();
    }
    context.config.pi.executable = launcher.clone();
    let transcript = FixtureAsr
        .transcribe(AsrRequest {
            source_kind: None,
            context: context.clone(),
            segments: claim.segments,
        })
        .await
        .unwrap();
    let adapter = PiAdapter::new(paths);
    context.config.pi.max_input_bytes = 256000;
    let mut large = transcript.clone();
    large.segments[0].text = "SYNTHETIC.".repeat(15000);
    let error = adapter
        .summarize(SummaryRequest {
            context: context.clone(),
            transcript: large,
        })
        .await
        .unwrap_err();
    assert!(!error.retryable);
    assert!(error.message.contains("pi.node_executable"));
    context.config.pi.node_executable = Some(node.clone());
    assert!(adapter
        .summarize(SummaryRequest {
            context: context.clone(),
            transcript: transcript.clone()
        })
        .await
        .unwrap()
        .markdown
        .contains("managed stable minutes"));
    fs::set_permissions(&node, fs::Permissions::from_mode(0o777)).unwrap();
    assert_eq!(
        adapter
            .summarize(SummaryRequest {
                context: context.clone(),
                transcript: transcript.clone()
            })
            .await
            .unwrap_err()
            .code,
        ErrorCode::Permission
    );
    context.config.pi.node_executable = Some(runtime.join("python"));
    assert_eq!(
        adapter
            .summarize(SummaryRequest {
                context,
                transcript
            })
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
}

#[tokio::test]
async fn unsupported_pi_flags_and_bad_interpreter_are_actionable_nonretryable_without_stderr() {
    let (_t, p, mut s, _) = setup(
        "/bin/cat >/dev/null; printf 'unknown option --no-tools SECRET_PRIVATE_PATH' >&2; exit 2",
    );
    let pi = PiAdapter::new(p);
    run_one(&mut s, &FixtureAsr, &pi, 10000).await.unwrap();
    let failed = run_one(&mut s, &NoAsr, &pi, 10000).await.unwrap().unwrap();
    assert_eq!(failed.status, TaskStatus::Blocked);
    let error = failed.error.unwrap();
    assert!(!error.retryable);
    assert!(error.message.contains("unsupported"));
    assert!(!error.message.contains("SECRET"));
    let (_t, p, mut s, _) = setup("exit 0");
    let claim = s
        .claim(chrono::Utc::now().timestamp_millis(), 10000)
        .unwrap()
        .unwrap();
    fs::write(
        &claim.context.config.pi.executable,
        "#!/missing-fixture-interpreter\n",
    )
    .unwrap();
    let transcript = FixtureAsr
        .transcribe(AsrRequest {
            source_kind: None,
            context: claim.context.clone(),
            segments: claim.segments,
        })
        .await
        .unwrap();
    let error = PiAdapter::new(p)
        .summarize(SummaryRequest {
            context: claim.context,
            transcript,
        })
        .await
        .unwrap_err();
    assert!(!error.retryable);
    assert!(error.message.contains("pi.executable"));
    assert!(!error.message.contains("missing-fixture-interpreter"));
}

#[tokio::test]
async fn pi_and_node_intermediate_writable_links_block_before_child_execution() {
    use std::os::unix::fs::symlink;
    for node_case in [false, true] {
        for mode in [0o770, 0o777] {
            let (t, p, mut store, task) = setup("exit 127");
            let claim = store
                .claim(chrono::Utc::now().timestamp_millis(), 10000)
                .unwrap()
                .unwrap();
            let mut pi_config = claim.context.config.pi.clone();
            let transcript = FixtureAsr
                .transcribe(AsrRequest {
                    source_kind: None,
                    context: claim.context.clone(),
                    segments: claim.segments.clone(),
                })
                .await
                .unwrap();
            store
                .commit_transcript(&claim, &transcript, chrono::Utc::now().timestamp_millis())
                .unwrap();
            let pi = PiAdapter::new(p);
            assert_eq!(
                run_one(&mut store, &NoAsr, &pi, 10000)
                    .await
                    .unwrap()
                    .unwrap()
                    .status,
                TaskStatus::Blocked
            );
            let trace = t.path().join("must-not-execute");
            let body = format!(
                "#!/bin/sh\nprintf x >> '{}'\n/bin/cat >/dev/null\nprintf '# unsafe fixture'\n",
                trace.display()
            );
            let target = if node_case {
                let target = t.path().join("safe-node");
                fs::write(&target, &body).unwrap();
                fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
                fs::write(&pi_config.executable, "#!/usr/bin/env node\n").unwrap();
                target
            } else {
                fs::write(&pi_config.executable, &body).unwrap();
                pi_config.executable.clone()
            };
            let unsafe_dir = t.path().join("unsafe");
            fs::create_dir(&unsafe_dir).unwrap();
            fs::set_permissions(&unsafe_dir, fs::Permissions::from_mode(mode)).unwrap();
            let name = if node_case { "node" } else { "pi" };
            symlink(target, unsafe_dir.join(name)).unwrap();
            let indirect = t.path().join("indirect");
            symlink(&unsafe_dir, &indirect).unwrap();
            if node_case {
                pi_config.node_executable = Some(indirect.join(name));
            } else {
                pi_config.executable = indirect.join(name);
            }
            let recovery = store
                .force_summary(task.id, "explicit-link-fixture", &pi_config)
                .unwrap();
            let blocked = run_one(&mut store, &NoAsr, &pi, 10000)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(blocked.id, recovery.id);
            assert_eq!(blocked.status, TaskStatus::Blocked);
            let error = blocked.error.unwrap();
            assert_eq!(error.code, ErrorCode::Permission);
            assert!(!error.retryable);
            assert!(!trace.exists(), "unsafe tool must never start");
            assert!(store.read_artifact(recovery.id, Stage::Summary).is_err());
            assert_eq!(
                store.read_artifact(task.id, Stage::Transcribe).unwrap(),
                store.read_artifact(recovery.id, Stage::Transcribe).unwrap()
            );
        }
    }
}

#[tokio::test]
async fn pi_and_node_trusted_multi_hop_links_and_parent_after_directory_link_keep_os_meaning() {
    use std::os::unix::fs::symlink;
    for node_case in [false, true] {
        for parent_after_link in [false, true] {
            let (t, p, mut store, _) = setup("/bin/cat >/dev/null; printf '# trusted chain'");
            let claim = store
                .claim(chrono::Utc::now().timestamp_millis(), 10000)
                .unwrap()
                .unwrap();
            let mut context = claim.context.clone();
            let safe = t.path().join("safe");
            omm::core::paths::private_dir(&safe).unwrap();
            omm::core::paths::private_dir(&safe.join("deep")).unwrap();
            symlink("../fake-pi", safe.join("pi")).unwrap();
            let runtime = safe.join("runtime-node");
            fs::write(&runtime, "#!/bin/sh\nexec /bin/sh \"$@\"\n").unwrap();
            fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
            symlink("runtime-node", safe.join("node")).unwrap();
            let relative = t.path().join("relative");
            symlink(&safe, &relative).unwrap();
            if parent_after_link {
                let prefix = t.path().join("prefix");
                omm::core::paths::private_dir(&prefix).unwrap();
                symlink("../safe/deep", prefix.join("dir")).unwrap();
                fs::write(prefix.join("pi"), "untrusted lexical decoy").unwrap();
                fs::set_permissions(prefix.join("pi"), fs::Permissions::from_mode(0o777)).unwrap();
                context.config.pi.executable = prefix.join("dir/./../pi");
                if node_case {
                    context.config.pi.node_executable = Some(prefix.join("dir/../node"));
                }
            } else {
                let entry = t.path().join("entry");
                symlink("relative/deep/../pi", &entry).unwrap();
                context.config.pi.executable = entry;
                if node_case {
                    let node = t.path().join("node");
                    symlink("relative/deep/../node", &node).unwrap();
                    context.config.pi.node_executable = Some(node);
                }
            }
            if node_case {
                fs::write(
                    &claim.context.config.pi.executable,
                    "#!/usr/bin/env node\n/bin/cat >/dev/null\nprintf '# trusted chain'\n",
                )
                .unwrap();
            }
            let transcript = FixtureAsr
                .transcribe(AsrRequest {
                    source_kind: None,
                    context: context.clone(),
                    segments: claim.segments,
                })
                .await
                .unwrap();
            assert!(PiAdapter::new(p)
                .summarize(SummaryRequest {
                    context,
                    transcript
                })
                .await
                .unwrap()
                .markdown
                .contains("trusted chain"));
        }
    }
}

#[tokio::test]
async fn pi_and_node_loop_missing_long_chain_and_overlong_paths_fail_safely_without_execution() {
    use std::os::unix::fs::symlink;
    for node_case in [false, true] {
        for case in 0..6 {
            let (t, p, mut store, _) = setup("/bin/cat >/dev/null; printf '# must not run'");
            let claim = store
                .claim(chrono::Utc::now().timestamp_millis(), 10000)
                .unwrap()
                .unwrap();
            let mut context = claim.context.clone();
            let name = if node_case { "node" } else { "pi" };
            let entry = t.path().join(name);
            let selected = match case {
                0 => {
                    symlink(name, &entry).unwrap();
                    entry
                }
                1 => {
                    symlink("absent", &entry).unwrap();
                    entry
                }
                2 => {
                    symlink("hop0", &entry).unwrap();
                    for i in 0..41 {
                        symlink(
                            if i == 40 {
                                claim.context.config.pi.executable.clone()
                            } else {
                                t.path().join(format!("hop{}", i + 1))
                            },
                            t.path().join(format!("hop{i}")),
                        )
                        .unwrap();
                    }
                    entry
                }
                3 => t.path().join("a".repeat(4097)).join(name),
                4 => claim.context.config.pi.executable.join("..").join(name),
                _ => t.path().join(vec!["a"; 300].join("/")).join(name),
            };
            if node_case {
                context.config.pi.node_executable = Some(selected);
            } else {
                context.config.pi.executable = selected;
            }
            let transcript = FixtureAsr
                .transcribe(AsrRequest {
                    source_kind: None,
                    context: context.clone(),
                    segments: claim.segments,
                })
                .await
                .unwrap();
            let error = PiAdapter::new(p)
                .summarize(SummaryRequest {
                    context,
                    transcript,
                })
                .await
                .unwrap_err();
            assert!(!error.retryable);
            assert!(!error.message.contains(&t.path().display().to_string()));
        }
    }
}

#[tokio::test]
async fn pi_node_colon_parent_is_rejected_without_starting_pi_or_unvalidated_node() {
    let (t, p, mut store, _) = setup("exit 81");
    let claim = store
        .claim(chrono::Utc::now().timestamp_millis(), 10000)
        .unwrap()
        .unwrap();
    let mut context = claim.context.clone();
    let root = t.path().canonicalize().unwrap();
    let pi_trace = root.join("pi.trace");
    let node_trace = root.join("node.trace");
    let evil = root.join("unvalidated");
    fs::create_dir(&evil).unwrap();
    fs::set_permissions(&evil, fs::Permissions::from_mode(0o777)).unwrap();
    let script = format!(
        "#!/bin/sh\nprintf x >> '{}'\n/bin/cat >/dev/null\nprintf '# UNVALIDATED_NODE_EXECUTED'\n",
        node_trace.display()
    );
    fs::write(evil.join("node"), &script).unwrap();
    fs::set_permissions(evil.join("node"), fs::Permissions::from_mode(0o777)).unwrap();
    let node = std::path::PathBuf::from(format!(
        "{}:{}/node",
        root.join("safe").display(),
        evil.display()
    ));
    fs::create_dir_all(node.parent().unwrap()).unwrap();
    fs::write(&node, &script).unwrap();
    fs::set_permissions(&node, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        &context.config.pi.executable,
        format!(
            "#!/bin/sh\nprintf x >> '{}'\nexec /usr/bin/env node \"$@\"\n",
            pi_trace.display()
        ),
    )
    .unwrap();
    context.config.pi.node_executable = Some(node);
    let transcript = FixtureAsr
        .transcribe(AsrRequest {
            source_kind: None,
            context: context.clone(),
            segments: claim.segments,
        })
        .await
        .unwrap();
    let error = PiAdapter::new(p)
        .summarize(SummaryRequest {
            context,
            transcript,
        })
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "pi.node_executable cannot be represented in isolated PATH"
    );
    assert!(!error.retryable);
    assert!(!pi_trace.exists());
    assert!(!node_trace.exists());
}

#[tokio::test]
async fn pi_colon_executable_is_direct_and_normal_node_path_contains_only_validated_entries() {
    for node_case in [false, true] {
        let (t, p, mut store, _) = setup("exit 81");
        let claim = store
            .claim(chrono::Utc::now().timestamp_millis(), 10000)
            .unwrap()
            .unwrap();
        let mut context = claim.context.clone();
        let pi_dir = t.path().join("pi:stable");
        omm::core::paths::private_dir(&pi_dir).unwrap();
        let executable = pi_dir.join("pi");
        let body =
            "/bin/cat >/dev/null\nprintf 'PATH=%s\\n' \"$PATH\"\nprintf '# trusted direct Pi'\n";
        fs::write(
            &executable,
            format!(
                "{}{}",
                if node_case {
                    "#!/usr/bin/env node\n"
                } else {
                    "#!/bin/sh\n"
                },
                body
            ),
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        context.config.pi.executable = executable;
        let mut expected = vec![
            std::path::PathBuf::from("/usr/bin"),
            std::path::PathBuf::from("/bin"),
        ];
        if node_case {
            let runtime = t.path().join("trusted-runtime");
            omm::core::paths::private_dir(&runtime).unwrap();
            let node = runtime.join("node");
            fs::write(&node, "#!/bin/sh\nexec /bin/sh \"$@\"\n").unwrap();
            fs::set_permissions(&node, fs::Permissions::from_mode(0o700)).unwrap();
            context.config.pi.node_executable = Some(node);
            expected.insert(0, runtime);
        }
        let transcript = FixtureAsr
            .transcribe(AsrRequest {
                source_kind: None,
                context: context.clone(),
                segments: claim.segments,
            })
            .await
            .unwrap();
        let summary = PiAdapter::new(p)
            .summarize(SummaryRequest {
                context,
                transcript,
            })
            .await
            .unwrap();
        assert!(summary.markdown.contains("trusted direct Pi"));
        let observed = summary
            .markdown
            .lines()
            .next()
            .unwrap()
            .strip_prefix("PATH=")
            .unwrap();
        assert_eq!(
            std::env::split_paths(observed).collect::<Vec<_>>(),
            expected
        );
    }
}

struct NoAsr;
#[async_trait]
impl AsrAdapter for NoAsr {
    async fn transcribe(&self, _: AsrRequest) -> Result<Transcript> {
        panic!("summary recovery must not run ASR")
    }
}
#[tokio::test]
async fn missing_pi_path_is_actionable_and_force_summary_preserves_old_attempt_and_transcript() {
    let (t, paths, mut store, task) = setup("/bin/cat >/dev/null; printf '# restored'");
    let claim = store
        .claim(chrono::Utc::now().timestamp_millis(), 10000)
        .unwrap()
        .unwrap();
    let old_config = claim.context.config.clone();
    let active_before = serde_json::to_vec(&store.task(task.id).unwrap()).unwrap();
    let mut future_pi = old_config.pi.clone();
    future_pi.executable = t.path().join("future-stable-pi");
    assert!(store
        .force_summary(task.id, "new-tool-config", &future_pi)
        .is_err());
    assert_eq!(
        active_before,
        serde_json::to_vec(&store.task(task.id).unwrap()).unwrap()
    );
    let transcript = FixtureAsr
        .transcribe(AsrRequest {
            source_kind: None,
            context: claim.context.clone(),
            segments: claim.segments.clone(),
        })
        .await
        .unwrap();
    store
        .commit_transcript(&claim, &transcript, chrono::Utc::now().timestamp_millis())
        .unwrap();
    let before = store.read_artifact(task.id, Stage::Transcribe).unwrap();
    fs::remove_file(&old_config.pi.executable).unwrap();
    let pi = PiAdapter::new(paths.clone());
    let blocked = run_one(&mut store, &NoAsr, &pi, 10000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(blocked.status, TaskStatus::Blocked);
    assert!(blocked.error.unwrap().message.contains("pi.executable"));
    let stable = t.path().join("stable-pi");
    fs::write(
        &stable,
        "#!/bin/sh\n/bin/cat >/dev/null\nprintf x >> \"$0.count\"\nprintf '# restored'\n",
    )
    .unwrap();
    fs::set_permissions(&stable, fs::Permissions::from_mode(0o700)).unwrap();
    let mut fixed = old_config.pi.clone();
    fixed.executable = stable.clone();
    let recovery = store
        .force_summary(task.id, "recovered-tool-reference", &fixed)
        .unwrap();
    assert_eq!(
        before,
        store.read_artifact(recovery.id, Stage::Transcribe).unwrap()
    );
    assert_eq!(store.task(task.id).unwrap().config_hash, task.config_hash);
    let result = run_one(&mut store, &NoAsr, &pi, 10000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.status, TaskStatus::Completed);
    assert!(run_one(&mut store, &NoAsr, &pi, 10000)
        .await
        .unwrap()
        .is_none());
    assert_eq!(fs::read(stable.with_extension("count")).unwrap(), b"x");
    assert_eq!(
        before,
        store.read_artifact(task.id, Stage::Transcribe).unwrap()
    );
}

#[tokio::test]
async fn reapers_observe_nested_cleanup_failure_registered_during_drain() {
    use std::{future::Future, task::Poll};
    let reapers = omm::core::process::Reapers::default();
    let first = reapers.register_cleanup();
    let drain = reapers.drain();
    tokio::pin!(drain);
    std::future::poll_fn(|cx| {
        assert!(drain.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let nested = reapers.register_cleanup();
    nested.send_replace(Some(false));
    first.send_replace(Some(true));
    assert!(
        drain.await.is_err(),
        "nested unconfirmed child must not be removed as an observed wait"
    );
}

#[tokio::test]
async fn summary_commit_and_notification_boundary_waits_for_adapter_drain() {
    struct ObserveDrain(Paths);
    #[async_trait]
    impl SummaryAdapter for ObserveDrain {
        async fn summarize(&self, _: SummaryRequest) -> Result<Summary> {
            Ok(Summary {
                schema_version: 1,
                markdown: "fixture minutes".into(),
                provider: "fixture-provider".into(),
                model: "fixture-model".into(),
                prompt_version: "1".into(),
            })
        }
        async fn drain(&self) -> Result<()> {
            let store = Store::open(&self.0)?;
            let tasks = store.tasks()?;
            assert!(
                tasks
                    .iter()
                    .all(|task| task.status != TaskStatus::Completed),
                "public completion must not precede owned drain"
            );
            Ok(())
        }
    }
    let (_temp, paths, mut store, task) = setup("exit 81");
    let pi = ObserveDrain(paths);
    run_one(&mut store, &FixtureAsr, &pi, 10000).await.unwrap();
    assert_eq!(
        run_one(&mut store, &FixtureAsr, &pi, 10000)
            .await
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Completed
    );
    assert!(store.read_artifact(task.id, Stage::Summary).is_ok());
}

#[tokio::test]
async fn cancelled_owned_pi_is_waited_before_settlement_and_retains_unverified_workspace() {
    use tokio::io::AsyncReadExt;
    let (_temp, paths, mut store, task) = setup("exit 81");
    let socket = _temp.path().canonicalize().unwrap().join("ready.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let script = format!("#!/bin/sh\nexec /usr/bin/python3 -c 'import socket,sys; sys.stdout.write(\"synthetic partial output\"); sys.stdout.flush(); sys.stderr.write(\"synthetic diagnostic\"); sys.stderr.flush(); s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.sendall(b\"ready\"); s.recv(1)' '{}'\n", socket.display());
    fs::write(_temp.path().join("fake-pi"), script).unwrap();
    let pi = PiAdapter::new(paths.clone());
    run_one(&mut store, &FixtureAsr, &pi, 10000).await.unwrap();
    let mut observer = Store::open(&paths).unwrap();
    let operation = run_one(&mut store, &FixtureAsr, &pi, 10000);
    tokio::pin!(operation);
    let (mut stream, _) = tokio::select! {
        accepted = listener.accept() => accepted.unwrap(),
        result = &mut operation => panic!("Pi finished before readiness: {result:?}"),
    };
    let mut ready = [0; 5];
    stream.read_exact(&mut ready).await.unwrap();
    observer.desktop_cancel(task.id).unwrap();
    let cancelled = operation.await.unwrap().unwrap();
    assert_eq!(cancelled.status, TaskStatus::Blocked);
    let mut bytes = vec![];
    stream.read_to_end(&mut bytes).await.unwrap();
    pi.drain().await.unwrap();
    let directory = fs::read_dir(paths.cache.join("pi"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let owner: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("owner.json")).unwrap()).unwrap();
    let pid = owner["child_pid"].as_u64().unwrap() as i32;
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD),
        "owned child must already have been waited, not merely marked terminal"
    );
    eprintln!("synthetic Pi pid={pid}: registered drain completed, kernel reports ECHILD, private workspace retained; background unverified");
    assert!(observer.read_artifact(task.id, Stage::Summary).is_err());
    assert!(observer.read_artifact(task.id, Stage::Transcribe).is_ok());
    assert!(observer
        .claim_summary(chrono::Utc::now().timestamp_millis(), 10000)
        .unwrap()
        .is_none());
    let report = omm::adapters::pi::recover_workspaces(&paths, &observer).unwrap();
    assert_eq!(report.removed, 0);
    assert_eq!(report.retained_unknown, 1);
}

// A synchronous guard retains the real child handle even if fixture setup unwinds.
struct PiPipeHolder {
    control: Option<std::process::ChildStdin>,
    waiter: Option<std::thread::JoinHandle<std::io::Result<std::process::Output>>>,
    pid: u32,
}
impl PiPipeHolder {
    fn finish(&mut self) -> std::io::Result<std::process::Output> {
        drop(self.control.take());
        let output = self
            .waiter
            .take()
            .unwrap()
            .join()
            .map_err(|_| std::io::Error::other("synthetic pipe holder waiter panicked"))??;
        eprintln!(
            "synthetic pipe holder pid={}: actual wait={}, stderr={}",
            self.pid,
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(output)
    }
}
impl Drop for PiPipeHolder {
    fn drop(&mut self) {
        if self.waiter.is_some() {
            if let Err(error) = self.finish() {
                eprintln!("synthetic pipe holder wait failed: {error}");
            }
        }
    }
}

// Transfer actual Pi stdout to an owned bounded holder, not an un-waitable grandchild.
async fn cancelled_pi_pipe_hold_outlasts_lease(invalidate_lease: bool) {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    const LEASE_MS: i64 = 600;
    let (temp, paths, mut store, task) = setup("exit 81");
    let root = temp.path().canonicalize().unwrap();
    let ready_path = root.join("ready.sock");
    let hold_path = root.join("hold.sock");
    let diagnostic_path = root.join("synthetic-pi-diagnostic.txt");
    let listener = tokio::net::UnixListener::bind(&ready_path).unwrap();
    let holder_script = r#"import array,os,select,socket,sys
server=socket.socket(socket.AF_UNIX)
server.bind(sys.argv[1]); server.listen(1); server.settimeout(4)
ready=socket.socket(socket.AF_UNIX); ready.connect(sys.argv[2]); ready.sendall(b'H')
s,_=server.accept()
_,anc,_,_=s.recvmsg(1,socket.CMSG_SPACE(4))
fds=array.array('i'); fds.frombytes(anc[0][2][:4])
s.sendall(b'K')
if select.select([sys.stdin],[],[],4)[0]: sys.stdin.buffer.read(1)
os.close(fds[0]); s.close(); server.close()
ready.sendall(b'D'); ready.close()
"#;
    let pi_script = root.join("synthetic-pi.py");
    fs::write(
        &pi_script,
        r#"import array,os,socket,sys,traceback
try:
    s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])
    s.sendmsg([b'x'],[(socket.SOL_SOCKET,socket.SCM_RIGHTS,array.array('i',[1]))])
    assert s.recv(1)==b'K'
    r=socket.socket(socket.AF_UNIX); r.connect(sys.argv[2]); r.sendall(b'P'); r.recv(1)
except BaseException:
    with os.fdopen(os.open(sys.argv[3],os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600),'w') as output:
        traceback.print_exc(file=output)
    raise
"#,
    )
    .unwrap();
    fs::write(
        root.join("fake-pi"),
        format!(
            "#!/bin/sh\nexec /usr/bin/python3 '{}' '{}' '{}' '{}' \"$@\"\n",
            pi_script.display(),
            hold_path.display(),
            ready_path.display(),
            diagnostic_path.display()
        ),
    )
    .unwrap();
    let pi = PiAdapter::new(paths.clone());
    run_one(&mut store, &FixtureAsr, &pi, 10000).await.unwrap();
    let mut observer = Store::open(&paths).unwrap();
    let db = rusqlite::Connection::open(paths.state.join("state.sqlite")).unwrap();
    let expires = || {
        db.query_row(
            "SELECT expires FROM tasks WHERE id=?1",
            [task.id.to_string()],
            |row| row.get::<_, i64>(0),
        )
    };
    let mut child = std::process::Command::new("/usr/bin/python3")
        .args(["-c", holder_script])
        .arg(&hold_path)
        .arg(&ready_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut holder = PiPipeHolder {
        control: child.stdin.take(),
        pid: child.id(),
        waiter: Some(std::thread::spawn(move || child.wait_with_output())),
    };
    // All fallible setup and adapter work return through the common cleanup below.
    let phase: Result<_> = async {
        let (mut holder_ready, _) = tokio::time::timeout(
            std::time::Duration::from_secs(4), listener.accept(),
        ).await.map_err(|_| OmmError::unavailable("synthetic holder readiness timed out"))??;
        let mut byte = [0];
        holder_ready.read_exact(&mut byte).await?;
        if &byte != b"H" { return Err(OmmError::invalid("synthetic holder readiness")); }
        let notifications = omm::core::notifications::NotificationSink::default();
        let operation = omm::core::pipeline::run_summary_one_notifying(
            &mut store, &FixtureAsr, &pi, LEASE_MS, &notifications,
        );
        tokio::pin!(operation);
        let (mut ready, _) = tokio::select! {
            accepted = listener.accept() => accepted?,
            result = &mut operation => return Err(OmmError::invalid(&format!("Pi finished before pipe readiness: {result:?}"))),
        };
        ready.read_exact(&mut byte).await?;
        if &byte != b"P" { return Err(OmmError::invalid("synthetic Pi readiness")); }
        let before = expires()?;
        if invalidate_lease {
            db.execute("UPDATE tasks SET expires=0 WHERE id=?1", [task.id.to_string()])?;
        } else {
            observer.desktop_cancel(task.id)?;
        }
        // Deliberately expose two lease windows while EOF is held by the test.
        let pending = tokio::time::timeout(
            std::time::Duration::from_millis(2 * LEASE_MS as u64), operation.as_mut(),
        ).await;
        let after = expires()?;
        let now = chrono::Utc::now().timestamp_millis();
        let still_draining = pending.is_err();
        drop(holder.control.take());
        let result = match pending { Ok(result) => result, Err(_) => operation.await };
        let mut eof = Vec::new();
        ready.read_to_end(&mut eof).await?;
        holder_ready.read_to_end(&mut eof).await?;
        Ok((result, still_draining, before, after, now))
    }.await;
    // Release EOF, wait/drain the holder and Pi, then permit any assertion/panic.
    let holder_output = holder.finish();
    let drained = pi.drain().await;
    drop(listener);
    if let Ok(diagnostic) = fs::read_to_string(&diagnostic_path) {
        eprintln!(
            "synthetic Pi diagnostic: {}",
            diagnostic.chars().take(2000).collect::<String>()
        );
    }
    assert!(holder_output.unwrap().status.success());
    drained.unwrap();
    let (result, still_draining, lease_before, lease_after, observed_at) = phase.unwrap();
    assert!(still_draining, "renewal failure must not drop owned drain");
    assert!(observer.read_artifact(task.id, Stage::Summary).is_err());
    assert!(observer.read_artifact(task.id, Stage::Transcribe).is_ok());
    if invalidate_lease {
        assert_eq!(
            result.unwrap_err().code,
            ErrorCode::Conflict,
            "lost ownership must remain an error after cleanup"
        );
        return;
    }
    assert!(
        result.is_ok(),
        "cancellation must settle after held EOF: {result:?}"
    );
    eprintln!("durable lease before={lease_before} after={lease_after} observed_at={observed_at}; held Pi stdout across two lease windows");
    assert!(
        lease_after > lease_before && lease_after > observed_at,
        "drain must renew the actual durable lease"
    );
    assert_eq!(result.unwrap().unwrap().status, TaskStatus::Blocked);
    assert!(observer
        .claim_summary(chrono::Utc::now().timestamp_millis(), 10000)
        .unwrap()
        .is_none());
    let config: String = db
        .query_row(
            "SELECT config FROM tasks WHERE id=?1",
            [task.id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let config: ConfigSnapshot = serde_json::from_str(&config).unwrap();
    let session_id = uuid::Uuid::new_v4();
    let directory = paths.data.join("meetings").join(session_id.to_string());
    private_dir(&directory).unwrap();
    let audio = directory.join("segment.wav");
    fs::write(&audio, b"synthetic").unwrap();
    let next = store
        .enqueue(
            &SealedMeeting {
                session_id,
                title: "independent synthetic".into(),
                directory,
                sealed_at_ms: 1,
                segments: vec![AudioSegment {
                    capture: None,
                    path: audio,
                    duration_ms: 1000,
                    sha256: hash_bytes(b"synthetic"),
                }],
            },
            &config,
            false,
        )
        .unwrap();
    fs::write(
        root.join("fake-pi"),
        "#!/bin/sh\n/bin/cat >/dev/null\nprintf '# independent summary'\n",
    )
    .unwrap();
    run_one(&mut store, &FixtureAsr, &pi, 10000).await.unwrap();
    let notifications = omm::core::notifications::NotificationSink::default();
    let completed = omm::core::pipeline::run_summary_one_notifying(
        &mut store,
        &FixtureAsr,
        &pi,
        10000,
        &notifications,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(completed.id, next.id);
    assert_eq!(completed.status, TaskStatus::Completed);
}

#[tokio::test]
async fn cancelled_pi_renews_lease_while_owned_drain_waits_for_held_pipe() {
    cancelled_pi_pipe_hold_outlasts_lease(false).await;
}

#[tokio::test]
async fn cancelled_pi_finishes_owned_drain_even_when_renewal_loses_lease() {
    cancelled_pi_pipe_hold_outlasts_lease(true).await;
}

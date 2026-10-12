use omm::{
    core::{config::AppConfig, contracts::*, paths::*, store::Store},
    service::daemon::{Reply, Request},
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    process::{Child, Command, Output},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tempfile::TempDir;
use uuid::Uuid;
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[derive(Default)]
struct HelperState {
    session: Option<Uuid>,
    file: Option<PathBuf>,
    files: Vec<PathBuf>,
    recording: bool,
    starts: u32,
    deny: bool,
    wrong_identity: bool,
    quit_error: bool,
    retain_reply_endpoint: bool,
    reply_endpoint: Option<UnixStream>,
}
struct Fixture {
    _temp: TempDir,
    paths: Paths,
    runtime: PathBuf,
    state: Arc<Mutex<HelperState>>,
    stop: Arc<AtomicBool>,
    helper: Option<thread::JoinHandle<()>>,
    instance: Uuid,
    proxy: Mutex<Option<ChildGuard>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.proxy.lock().unwrap().take();
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.helper.take() {
            let _ = t.join();
        }
    }
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("omm-it-")
            .tempdir_in("/private/tmp")
            .unwrap();
        let root = temp.path().canonicalize().unwrap();
        let paths = Paths::resolve(&root.join("home"), &Default::default()).unwrap();
        paths.create_private_dirs().unwrap();
        let runtime = root.join("r");
        private_dir(&runtime).unwrap();
        private_dir(&runtime.join("omm/capture")).unwrap();
        let worker = root.join("fake-worker");
        let pi = root.join("fake-pi");
        fs::write(&worker,r#"#!/bin/sh
[ "$*" = '--protocol 1' ] || exit 70
printf x >> "$(/usr/bin/dirname "$0")/asr-count"
input=$(/bin/cat)
id=$(printf '%s' "$input" | /usr/bin/sed -En 's/.*"task_id":"([^"]+)".*/\1/p')
attempt=$(printf '%s' "$input" | /usr/bin/sed -En 's/.*"attempt":([0-9]+).*/\1/p')
printf '{"type":"result","schema_version":1,"task_id":"%s","attempt":%s,"transcript":{"schemaVersion":1,"modelVersion":"fixture-asr","durationMs":100,"segments":[{"source":"mic","startMs":0,"endMs":90,"text":"SYNTHETIC_TRANSCRIPT"}]}}\n' "$id" "$attempt"
"#).unwrap();
        fs::write(&pi,r#"#!/bin/sh
for flag in --print --no-tools --no-mcp --no-extensions --no-skills --no-session --no-context-files; do case " $* " in *" $flag "*) ;; *) exit 71;; esac; done
case "$*" in *SYNTHETIC_TRANSCRIPT*) exit 72;; esac
input=$(/bin/cat)
case "$input" in *segment-0000.wav*|*state.sqlite*) exit 73;; esac
printf '# Synthetic minutes\nDecision at 0ms.\n'
"#).unwrap();
        for p in [&worker, &pi] {
            fs::set_permissions(p, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let cfg = AppConfig {
            worker_executable: worker,
            asr: AsrConfig {
                model_version: "fixture-asr".into(),
                timeout_ms: 2000,
                ..Default::default()
            },
            pi: PiConfig {
                executable: pi,
                provider: "fixture-cloud".into(),
                model: "fixture-model".into(),
                prompt_version: "v1".into(),
                timeout_ms: 2000,
                max_input_bytes: 1024,
                max_output_bytes: 1024,
            },
            poll_ms: 100,
            ..Default::default()
        };
        cfg.save(&paths).unwrap();
        let instance = Uuid::new_v4();
        Store::open(&paths)
            .unwrap()
            .set_state("capture_instance", &instance)
            .unwrap();
        let socket = root.join("helper-broker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(HelperState::default()));
        let (stop2, state2) = (stop.clone(), state.clone());
        let helper = thread::spawn(move || {
            while !stop2.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let mut bytes = vec![];
                        stream.read_to_end(&mut bytes).unwrap();
                        let req: Value = serde_json::from_slice(&bytes).unwrap();
                        let mut s = state2.lock().unwrap();
                        let response = match req["action"].as_str().unwrap() {
                            "status" => {
                                json!({"quitError":s.quit_error,"captureIPCVersion":1,"instance_id":if s.wrong_identity {Uuid::nil()} else {instance},"recording":s.recording,"session_id":s.session.map(|id|id.to_string()).unwrap_or_default(),"file":s.file.as_ref().map(|p|p.to_str().unwrap()).unwrap_or(""),"sysReady":!s.deny,"micReady":!s.deny})
                            }
                            "start" => {
                                if s.deny {
                                    json!({"error":"mic_capture_not_ready"})
                                } else {
                                    let session =
                                        Uuid::parse_str(req["session_id"].as_str().unwrap())
                                            .unwrap();
                                    let file = PathBuf::from(req["output_dir"].as_str().unwrap())
                                        .join(req["filename"].as_str().unwrap());
                                    let spec = hound::WavSpec {
                                        channels: 2,
                                        sample_rate: 48000,
                                        bits_per_sample: 16,
                                        sample_format: hound::SampleFormat::Int,
                                    };
                                    let mut wav = hound::WavWriter::create(&file, spec).unwrap();
                                    for _ in 0..9600 {
                                        wav.write_sample(0i16).unwrap();
                                    }
                                    wav.finalize().unwrap();
                                    s.starts += 1;
                                    s.session = Some(session);
                                    s.file = Some(file.clone());
                                    s.files = vec![file.clone()];
                                    s.recording = true;
                                    json!({"instance_id":instance,"session_id":session,"status":"recording","file":file})
                                }
                            }
                            "stop" => {
                                s.recording = false;
                                json!({"instance_id":instance,"session_id":s.session,"status":"stopped","files":s.files,"file":s.file,"duration":0.1})
                            }
                            "windows" => json!({"windows":[]}),
                            "audio_devices" => json!({"input":[],"default_input_uid":""}),
                            _ => panic!("unexpected fixture operation"),
                        };
                        if s.retain_reply_endpoint {
                            s.reply_endpoint = Some(stream.try_clone().unwrap());
                        }
                        drop(s);
                        let _ = stream.write_all(&serde_json::to_vec(&response).unwrap());
                        // macOS accept + CLOEXEC is non-atomic: a parallel spawn
                        // can inherit this endpoint. Signal EOF even if a copy lives.
                        if let Err(error) = stream.shutdown(std::net::Shutdown::Write) {
                            assert_eq!(error.kind(), std::io::ErrorKind::NotConnected, "{error}");
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
        });
        let fixture = Self {
            _temp: temp,
            paths,
            runtime,
            state,
            stop,
            helper: Some(helper),
            instance,
            proxy: Mutex::new(None),
        };
        fixture.start_proxy();
        fixture
    }
    fn start_proxy(&self) {
        let mut proxy = self.proxy.lock().unwrap();
        if proxy
            .as_mut()
            .is_some_and(|child| child.0.try_wait().unwrap().is_none())
        {
            return;
        }
        *proxy = Some(ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "core_capture_helper_proxy",
                    "--ignored",
                    "--nocapture",
                ])
                .env_clear()
                .env(
                    "OMM_TEST_CAPTURE_SOCKET",
                    self.runtime.join("omm/capture/audio_daemon.sock"),
                )
                .env(
                    "OMM_TEST_CAPTURE_BROKER",
                    self._temp.path().join("helper-broker.sock"),
                )
                .env("OMM_TEST_CAPTURE_INSTANCE", self.instance.to_string())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::inherit())
                .spawn()
                .unwrap(),
        ));
        await_condition(|| self.runtime.join("omm/capture/audio_daemon.sock").exists());
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_omm"));
        cmd.env_clear()
            .env(
                "HOME",
                self.paths.config.parent().unwrap().parent().unwrap(),
            )
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .args(args)
            .arg("--json")
            .arg("--non-interactive");
        cmd
    }
    fn invoke(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }
    fn ok(&self, args: &[&str]) -> Value {
        let o = self.invoke(args);
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stdout)
        );
        let value: Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        value["data"].clone()
    }
    fn error(&self, args: &[&str], code: i32) -> Value {
        let o = self.invoke(args);
        assert_eq!(
            o.status.code(),
            Some(code),
            "{}",
            String::from_utf8_lossy(&o.stdout)
        );
        serde_json::from_slice::<Value>(&o.stdout).unwrap()["error"].clone()
    }
    fn daemon(&self) -> ChildGuard {
        if self.helper.is_some() {
            self.start_proxy();
        }
        let child = ChildGuard(
            self.command(&["service", "run"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        for _ in 0..100 {
            if self.invoke(&["service", "status"]).status.success() {
                return child;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("fixture service did not become ready");
    }
    fn callback(&self, session: Uuid) -> Reply {
        let mut stream = UnixStream::connect(self.runtime.join("omm/service.sock")).unwrap();
        stream
            .write_all(
                &serde_json::to_vec(&Request {
                    schema_version: 1,
                    command: "capture_auto_stop".into(),
                    title: None,
                    session_id: Some(session),
                    instance_id: Some(self.instance),
                })
                .unwrap(),
            )
            .unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut bytes = vec![];
        stream.read_to_end(&mut bytes).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
}
#[test]
fn real_cli_service_capture_worker_and_pi_queue_round_trip_without_audio_upload_flag() {
    let f = Fixture::new();
    assert_eq!(
        f.error(&["record", "auto", "enable"], 3)["code"],
        "consent_required"
    );
    f.ok(&["record", "auto", "enable", "--confirm-recording-consent"]);
    f.ok(&["record", "auto", "disable"]);
    let mut daemon = f.daemon();
    for args in [
        &["paths"][..],
        &["doctor"],
        &["init", "--check"],
        &["permissions", "check"],
        &["record", "status"],
        &["config", "show"],
        &["model", "list"],
    ] {
        f.ok(args);
    }
    assert_eq!(
        f.error(&["permissions", "request"], 3)["code"],
        "interaction_required"
    );
    assert_eq!(
        f.error(&["record", "auto", "enable"], 3)["code"],
        "consent_required"
    );
    f.ok(&["config", "set", "pi.model", "fixture-model"]);
    f.error(&["config", "set", "api_key", "SECRET_FIXTURE"], 2);
    f.error(&["model", "verify", "silero-vad"], 3);
    f.error(&["model", "remove", "silero-vad"], 3);
    let started = f.ok(&["record", "start", "--title", "Synthetic meeting"]);
    let session = started["session_id"].as_str().unwrap().to_owned();
    f.error(&["record", "start"], 6);
    assert_eq!(f.state.lock().unwrap().starts, 1);
    let sealed = f.ok(&["record", "stop"]);
    let task = sealed["task"]["id"].as_str().unwrap().to_owned();
    f.ok(&["task", "wait", &task, "--timeout", "1m"]);
    f.ok(&["task", "show", &task]);
    f.ok(&["task", "list", "--status", "completed"]);
    let list = f.ok(&[
        "meeting",
        "list",
        "--from",
        "2026-01-01",
        "--query",
        "Synthetic",
        "--status",
        "completed",
    ]);
    assert_eq!(list.as_array().unwrap().len(), 1);
    f.ok(&["meeting", "show", &session]);
    assert!(
        f.ok(&["meeting", "read", &session, "--artifact", "summary"])["content"]
            .as_str()
            .unwrap()
            .contains("Synthetic minutes")
    );
    f.ok(&["meeting", "read", &session, "--artifact", "transcript"]);
    assert_eq!(f.ok(&["process", &session, "--stage", "all"])["id"], task);
    f.ok(&[
        "process",
        &session,
        "--stage",
        "summary",
        "--wait",
        "--timeout",
        "1s",
    ]);
    let forced = f.ok(&["process", &session, "--force"]);
    assert_ne!(forced["id"], task);
    assert_eq!(forced["attempt"], 2);
    f.ok(&[
        "task",
        "wait",
        forced["id"].as_str().unwrap(),
        "--timeout",
        "1m",
    ]);
    let mut config = AppConfig::load(&f.paths).unwrap();
    config.asr.model_version = "new-asr-not-used".into();
    config.pi.model = "new-summary-model".into();
    config.version = Uuid::new_v4().to_string();
    config.save(&f.paths).unwrap();
    let summary_force = f.ok(&["process", &session, "--stage", "summary", "--force"]);
    assert_eq!(summary_force["transcript_source"]["task_id"], forced["id"]);
    assert_eq!(
        summary_force["transcript_source"]["model_version"],
        "fixture-asr"
    );
    let imported = f.ok(&[
        "task",
        "wait",
        summary_force["id"].as_str().unwrap(),
        "--timeout",
        "1m",
    ]);
    assert_eq!(imported["summary_metadata"]["model"], "new-summary-model");
    assert_eq!(fs::read(f._temp.path().join("asr-count")).unwrap(), b"xx");
    let old = Uuid::parse_str(&task).unwrap();
    let store = Store::open(&f.paths).unwrap();
    let source = Uuid::parse_str(imported["id"].as_str().unwrap()).unwrap();
    assert!(store.read_artifact(old, Stage::Summary).is_ok());
    fs::write(
        store.artifact_path(source, Stage::Transcribe).unwrap(),
        b"CORRUPT_SYNTHETIC",
    )
    .unwrap();
    assert_eq!(
        f.error(&["process", &session, "--stage", "summary", "--force"], 4)["code"],
        "corrupt"
    );
    let m = Store::open(&f.paths)
        .unwrap()
        .meetings()
        .unwrap()
        .pop()
        .unwrap();
    assert!(m.segments[0].path.exists());
    f.ok(&["service", "stop"]);
    assert!(daemon.0.wait().unwrap().success());
    assert!(f.paths.state.join("state.sqlite").exists());
    assert!(m.segments[0].path.exists());
}
#[test]
fn service_kill_restart_reconciles_without_duplicate_start_and_callback_is_durable() {
    let f = Fixture::new();
    let mut daemon = f.daemon();
    let started = f.ok(&["record", "start"]);
    let session = Uuid::parse_str(started["session_id"].as_str().unwrap()).unwrap();
    daemon.0.kill().unwrap();
    daemon.0.wait().unwrap();
    let mut restarted = f.daemon();
    assert_eq!(f.ok(&["record", "status"])["recording"], true);
    assert_eq!(f.state.lock().unwrap().starts, 1);
    assert!(!f.callback(Uuid::new_v4()).ok);
    assert!(f.callback(session).ok);
    for _ in 0..100 {
        if !Store::open(&f.paths).unwrap().tasks().unwrap().is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let tasks = Store::open(&f.paths).unwrap().tasks().unwrap();
    assert_eq!(tasks.len(), 1);
    f.ok(&["task", "wait", &tasks[0].id.to_string(), "--timeout", "1m"]);
    f.ok(&["record", "stop"]);
    assert_eq!(Store::open(&f.paths).unwrap().tasks().unwrap().len(), 1);
    f.ok(&["service", "stop"]);
    assert!(restarted.0.wait().unwrap().success());
}
#[test]
fn refused_capture_permission_is_sanitized_and_never_a_successful_recording() {
    let f = Fixture::new();
    f.state.lock().unwrap().deny = true;
    let _daemon = f.daemon();
    let error = f.error(&["record", "start", "--title", "SECRET_FIXTURE"], 3);
    assert_eq!(error["code"], "permission");
    assert!(!error.to_string().contains("SECRET_FIXTURE"));
    assert!(!f.state.lock().unwrap().recording);
    assert!(Store::open(&f.paths).unwrap().tasks().unwrap().is_empty());
}

#[tokio::test]
async fn lark_fixture_requires_fresh_exact_room_and_identity_and_controller_confirms_start_end() {
    use omm::{
        adapters::{
            capture::{CapturePaths, MacCaptureAdapter},
            meeting::{rtc_room_matches, LarkAdapter},
        },
        core::lifecycle::Action,
        service::daemon::Controller,
    };
    let f = Fixture::new();
    for p in [
        f.paths.state.join("capture"),
        f.paths.state.join("logs"),
        f.paths.data.join("meetings"),
    ] {
        private_dir(&p).unwrap();
    }
    let capture = Arc::new(
        MacCaptureAdapter::new(
            CapturePaths {
                state: f.paths.state.join("capture"),
                logs: f.paths.state.join("logs"),
                media: f.paths.data.join("meetings"),
                ipc: f.runtime.join("omm/capture"),
            },
            f.instance,
            Duration::from_secs(2),
        )
        .unwrap(),
    );
    let mut cfg = AppConfig::load(&f.paths).unwrap();
    let rtc = f._temp.path().join("rtc");
    private_dir(&rtc).unwrap();
    let log = rtc.join("fixture_rtclog.log");
    fs::write(&log, "room_id:fixture-room123\n").unwrap();
    assert!(!rtc_room_matches(
        std::slice::from_ref(&rtc),
        "fixture-room12",
        std::time::SystemTime::now()
    ));
    let lark = f._temp.path().join("fake-lark");
    fs::write(&lark,r#"#!/bin/sh
case "$*" in
'auth status') printf '{"identities":{"user":{"openId":"synthetic-user"}}}' ;;
'vc +meeting-list-active --as user --format json') printf '{"data":{"meetings":[{"meeting_id":"fixture-room12","meeting_title":"Synthetic"}]}}' ;;
'vc meeting get --as user --meeting-id fixture-room12 --with-participants --user-id-type open_id --format json') printf '{"data":{"meeting":{"participants":[{"id":"synthetic-user","status":2}]}}}' ;;
*) exit 70;; esac
"#).unwrap();
    fs::set_permissions(&lark, fs::Permissions::from_mode(0o700)).unwrap();
    cfg.lark_executable = lark.clone();
    cfg.rtc_log_dirs = vec![rtc];
    let adapter = LarkAdapter::new(cfg.clone(), capture.clone());
    let weak = adapter.observe().await.unwrap();
    assert_eq!(weak.evidence, Evidence::Weak);
    assert_ne!(weak.state, MeetingState::Inactive);
    fs::write(log, "room_id:fixture-room12\n").unwrap();
    let mut o = adapter.observe().await.unwrap();
    assert_eq!(o.evidence, Evidence::Strong);
    cfg.recording_consent = true;
    let mut c = Controller::new(f.paths.clone(), capture, f.instance, &cfg);
    o.observed_at_ms = 1;
    assert_eq!(c.observation_action(&o, true).unwrap(), Action::None);
    o.observed_at_ms = 15000;
    assert_eq!(c.observation_action(&o, true).unwrap(), Action::None);
    o.observed_at_ms = 15001;
    assert_eq!(c.observation_action(&o, true).unwrap(), Action::Start);
    c.start("Synthetic".into(), o.source_id.clone())
        .await
        .unwrap();
    let id = c.recording().unwrap().unwrap().session_id;
    c.rotate().await.unwrap();
    assert_eq!(c.recording().unwrap().unwrap().session_id, id);
    assert_eq!(f.state.lock().unwrap().starts, 2);
    let mut ended = o.clone();
    ended.state = MeetingState::Inactive;
    ended.evidence = Evidence::Strong;
    ended.observed_at_ms = 20000;
    assert_eq!(c.observation_action(&ended, true).unwrap(), Action::None);
    let mut unknown = ended.clone();
    unknown.state = MeetingState::Unknown;
    unknown.observed_at_ms = 49000;
    assert_eq!(c.observation_action(&unknown, true).unwrap(), Action::None);
    ended.observed_at_ms = 50000;
    assert_eq!(c.observation_action(&ended, true).unwrap(), Action::None);
    ended.observed_at_ms = 80000;
    assert_eq!(c.observation_action(&ended, true).unwrap(), Action::Stop);
    c.seal(false).await.unwrap();
    let sealed = Store::open(&f.paths)
        .unwrap()
        .meetings()
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(sealed.segments.len(), 2);
    fs::write(
        lark,
        "#!/bin/sh\nprintf 'SECRET_SYNTHETIC_AUTH_ERROR' >&2\nexit 1\n",
    )
    .unwrap();
    let failed = adapter.observe().await.unwrap();
    assert_eq!(failed.state, MeetingState::Unknown);
}

fn configure_lark(f: &Fixture, slow: bool) {
    let root = f._temp.path();
    let exe = root.join("fake-lark");
    fs::write(&exe, format!(r#"#!/bin/sh
root=$(/usr/bin/dirname "$0")
if [ "$1" = auth ]; then printf '{{"identities":{{"user":{{"openId":"fixture-user"}}}}}}'; exit; fi
if [ "$2" = +meeting-list-active ]; then
 phase=$(/bin/cat "$root/phase")
 [ "$phase" != unknown ] || exit 1
 printf '{{"data":{{"meetings":[{{"meeting_id":"%s","meeting_title":"Synthetic"}}]}}}}' "$phase"; exit
fi
echo $$ > "$root/observer-pid"
if [ {} = true ]; then touch "$root/observer-started"; exec /bin/sleep 20; fi
printf '{{"data":{{"meeting":{{"participants":[{{"id":"fixture-user","status":2}}]}}}}}}'
"#, slow)).unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(root.join("phase"), "A").unwrap();
    fs::write(root.join("fixture_rtclog.log"), "room_id:A room_id:B").unwrap();
    let mut cfg = AppConfig::load(&f.paths).unwrap();
    cfg.lark_executable = exe;
    cfg.rtc_log_dirs = vec![root.to_owned()];
    cfg.auto_record = true;
    cfg.recording_consent = true;
    cfg.start_delay_ms = 400;
    cfg.end_grace_ms = 1000;
    cfg.save(&f.paths).unwrap();
}
#[track_caller]
fn await_condition(mut predicate: impl FnMut() -> bool) {
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(
            std::time::Instant::now() < until,
            "fixture condition timed out"
        );
        thread::sleep(Duration::from_millis(20));
    }
}
#[test]
fn review_r1_01_service_switches_stable_strong_identity_but_not_jitter_unknown_or_manual() {
    let f = Fixture::new();
    configure_lark(&f, false);
    let mut service = f.daemon();
    await_condition(|| f.state.lock().unwrap().starts == 1);
    let first = f.state.lock().unwrap().session.unwrap();
    fs::write(f._temp.path().join("phase"), "B").unwrap();
    thread::sleep(Duration::from_millis(150));
    fs::write(f._temp.path().join("phase"), "unknown").unwrap();
    thread::sleep(Duration::from_millis(150));
    fs::write(f._temp.path().join("phase"), "A").unwrap();
    thread::sleep(Duration::from_millis(550));
    assert_eq!(f.state.lock().unwrap().session, Some(first));
    assert_eq!(f.state.lock().unwrap().starts, 1);
    fs::write(f._temp.path().join("phase"), "B").unwrap();
    await_condition(|| f.state.lock().unwrap().starts == 2);
    assert_ne!(f.state.lock().unwrap().session, Some(first));
    assert_eq!(f.ok(&["meeting", "list"]).as_array().unwrap().len(), 1);
    f.ok(&["record", "stop"]);
    let meetings = f.ok(&["meeting", "list"]);
    let meetings = meetings.as_array().unwrap();
    assert_eq!(meetings.len(), 2);
    assert!(meetings.iter().any(|meeting| meeting["sourceId"] == "A"));
    assert!(meetings.iter().any(|meeting| meeting["sourceId"] == "B"));
    assert_ne!(meetings[0]["id"], meetings[1]["id"]);
    thread::sleep(Duration::from_millis(650));
    assert_eq!(
        f.state.lock().unwrap().starts,
        2,
        "manual stop suppresses same B"
    );
    f.ok(&["record", "start"]);
    let manual = f.state.lock().unwrap().session;
    fs::write(f._temp.path().join("phase"), "A").unwrap();
    thread::sleep(Duration::from_millis(650));
    assert_eq!(
        f.state.lock().unwrap().session,
        manual,
        "manual recording has no source identity to switch"
    );
    f.ok(&["service", "stop"]);
    assert!(service.0.wait().unwrap().success());
}
#[test]
fn review_r1_02_slow_lark_never_blocks_status_stop_and_observer_is_reaped() {
    let f = Fixture::new();
    configure_lark(&f, true);
    let mut service = f.daemon();
    f.ok(&["record", "start"]);
    await_condition(|| f._temp.path().join("observer-started").exists());
    let pid: i32 = fs::read_to_string(f._temp.path().join("observer-pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let start = std::time::Instant::now();
    f.ok(&["service", "status"]);
    f.ok(&["record", "stop"]);
    assert!(
        start.elapsed() < Duration::from_millis(1500),
        "slow observation delayed controls: {:?}",
        start.elapsed()
    );
    f.ok(&["service", "stop"]);
    assert!(service.0.wait().unwrap().success());
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "observation child must be killed and waited before exit"
    );
}
#[test]
fn review_r1_04_normal_stop_cancels_pi_and_removes_workspace_after_child_exit() {
    let f = Fixture::new();
    let script = f._temp.path().join("fake-pi");
    // Atomic publication makes path existence mean the PID is ready to parse.
    fs::write(&script, "#!/bin/sh\n/bin/cat >/dev/null\npid_file=\"$(/usr/bin/dirname \"$0\")/pi-pid\"\necho $$ > \"$pid_file.tmp\"\n/bin/mv \"$pid_file.tmp\" \"$pid_file\"\nexec /bin/sleep 20\n").unwrap();
    let mut cfg = AppConfig::load(&f.paths).unwrap();
    cfg.pi.timeout_ms = 30000;
    cfg.save(&f.paths).unwrap();
    let mut service = f.daemon();
    f.ok(&["record", "start"]);
    f.ok(&["record", "stop"]);
    await_condition(|| f._temp.path().join("pi-pid").exists());
    let pid: i32 = fs::read_to_string(f._temp.path().join("pi-pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    f.ok(&["service", "stop"]);
    assert!(service.0.wait().unwrap().success());
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        fs::read_dir(f.paths.cache.join("pi")).unwrap().count(),
        0,
        "cancelled Pi workspace leaked"
    );
}
#[test]
fn review_r1_04_sigkill_keeps_live_orphan_then_reclaims_owned_workspace_after_exit() {
    let f = Fixture::new();
    fs::write(f._temp.path().join("fake-pi"), "#!/bin/sh\n/bin/cat >/dev/null\npid_file=\"$(/usr/bin/dirname \"$0\")/pi-pid\"\necho $$ > \"$pid_file.tmp\"\n/bin/mv \"$pid_file.tmp\" \"$pid_file\"\nexec /bin/sleep 3\n").unwrap();
    let mut cfg = AppConfig::load(&f.paths).unwrap();
    cfg.pi.timeout_ms = 30000;
    cfg.save(&f.paths).unwrap();
    let mut service = f.daemon();
    f.ok(&["record", "start"]);
    f.ok(&["record", "stop"]);
    await_condition(|| f._temp.path().join("pi-pid").exists());
    let pid: i32 = fs::read_to_string(f._temp.path().join("pi-pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    service.0.kill().unwrap();
    service.0.wait().unwrap();
    assert_eq!(fs::read_dir(f.paths.cache.join("pi")).unwrap().count(), 1);
    let mut restarted = f.daemon();
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        0,
        "bounded orphan should still be running"
    );
    assert_eq!(
        fs::read_dir(f.paths.cache.join("pi")).unwrap().count(),
        1,
        "do not remove workspace while orphan is live"
    );
    assert_eq!(
        f.ok(&["service", "status"])["piCacheRecovery"]["retainedLive"],
        1
    );
    await_condition(|| unsafe { libc::kill(pid, 0) } == -1);
    await_condition(|| fs::read_dir(f.paths.cache.join("pi")).unwrap().count() == 0);
    f.ok(&["service", "stop"]);
    assert!(restarted.0.wait().unwrap().success());
}
#[test]
fn review_r1_04_recovery_validates_task_state_lock_and_path_without_deleting_unknown() {
    use fs2::FileExt;
    use omm::adapters::pi::recover_workspaces;
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    fs::write(f._temp.path().join("fake-pi"), "#!/bin/sh\n/bin/cat >/dev/null\npid_file=\"$(/usr/bin/dirname \"$0\")/pi-pid\"\necho $$ > \"$pid_file.tmp\"\n/bin/mv \"$pid_file.tmp\" \"$pid_file\"\nexec /bin/sleep 2\n").unwrap();
    let mut service = f.daemon();
    f.ok(&["record", "start"]);
    f.ok(&["record", "stop"]);
    await_condition(|| f._temp.path().join("pi-pid").exists());
    let pid: i32 = fs::read_to_string(f._temp.path().join("pi-pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    service.0.kill().unwrap();
    service.0.wait().unwrap();
    await_condition(|| unsafe { libc::kill(pid, 0) } == -1);
    let directory = fs::read_dir(f.paths.cache.join("pi"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let marker = directory.join("owner.json");
    let original = fs::read(&marker).unwrap();
    let owner: Value = serde_json::from_slice(&original).unwrap();
    let store = Store::open(&f.paths).unwrap();
    let mut foreign = f.paths.clone();
    foreign.state = f._temp.path().join("other-state");
    let foreign_store = Store::open(&foreign).unwrap();
    assert_eq!(
        recover_workspaces(&foreign, &foreign_store)
            .unwrap()
            .retained_unknown,
        1
    );
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join("workspace.lock"))
        .unwrap();
    held.try_lock_exclusive().unwrap();
    assert_eq!(
        recover_workspaces(&f.paths, &store).unwrap().retained_live,
        1
    );
    drop(held);
    for (key, value) in [
        ("task_id", json!(Uuid::new_v4())),
        ("attempt", json!(999)),
        ("child_pid", Value::Null),
    ] {
        let mut invalid = owner.clone();
        invalid[key] = value;
        write_private_replace(&marker, &serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert_eq!(
            recover_workspaces(&f.paths, &store)
                .unwrap()
                .retained_unknown,
            1
        );
        assert!(directory.exists());
    }
    write_private_replace(&marker, b"invalid marker").unwrap();
    assert_eq!(
        recover_workspaces(&f.paths, &store)
            .unwrap()
            .retained_unknown,
        1
    );
    let outside = f._temp.path().join("protected-marker");
    write_private_new(&outside, &original).unwrap();
    fs::remove_file(&marker).unwrap();
    symlink(&outside, &marker).unwrap();
    assert_eq!(
        recover_workspaces(&f.paths, &store)
            .unwrap()
            .retained_unknown,
        1
    );
    assert_eq!(fs::read(&outside).unwrap(), original);
    fs::remove_file(&marker).unwrap();
    write_private_new(&marker, &original).unwrap();
    let saved = f._temp.path().join("saved-workspace");
    fs::rename(&directory, &saved).unwrap();
    symlink(&saved, &directory).unwrap();
    assert_eq!(
        recover_workspaces(&f.paths, &store)
            .unwrap()
            .retained_unknown,
        1
    );
    assert!(saved.join("home").is_dir());
    fs::remove_file(&directory).unwrap();
    fs::rename(&saved, &directory).unwrap();
    assert_eq!(recover_workspaces(&f.paths, &store).unwrap().removed, 1);
    assert!(!directory.exists());
    assert!(outside.exists());
}

#[tokio::test]
async fn review_r1_02_query_count_and_total_budget_return_unknown_and_reap() {
    use omm::adapters::{
        capture::{CapturePaths, MacCaptureAdapter},
        meeting::LarkAdapter,
    };
    let f = Fixture::new();
    configure_lark(&f, false);
    let cfg = AppConfig::load(&f.paths).unwrap();
    let meetings: Vec<Value> = (0..9)
        .map(|i| json!({"meeting_id":format!("fixture-{i}")}))
        .collect();
    let body = json!({"data":{"meetings":meetings}}).to_string();
    fs::write(
        &cfg.lark_executable,
        format!(
            r#"#!/bin/sh
root=$(/usr/bin/dirname "$0")
if [ "$1" = auth ]; then printf '{{"identities":{{"user":{{"openId":"fixture-user"}}}}}}'; exit; fi
if [ "$2" = +meeting-list-active ]; then printf '%s' '{}'; exit; fi
printf x >> "$root/query-count"
printf '{{"data":{{"meeting":{{"participants":[{{"id":"fixture-user","status":0}}]}}}}}}'
"#,
            body
        ),
    )
    .unwrap();
    for dir in [
        f.paths.state.join("capture"),
        f.paths.state.join("logs"),
        f.paths.data.join("meetings"),
    ] {
        private_dir(&dir).unwrap();
    }
    let capture = Arc::new(
        MacCaptureAdapter::new(
            CapturePaths {
                state: f.paths.state.join("capture"),
                logs: f.paths.state.join("logs"),
                media: f.paths.data.join("meetings"),
                ipc: f.runtime.join("omm/capture"),
            },
            f.instance,
            Duration::from_secs(2),
        )
        .unwrap(),
    );
    let bounded = LarkAdapter::new(cfg, capture.clone())
        .observe()
        .await
        .unwrap();
    assert_eq!(bounded.state, MeetingState::Unknown);
    assert_eq!(
        fs::read(f._temp.path().join("query-count")).unwrap().len(),
        8
    );
    configure_lark(&f, true);
    let start = std::time::Instant::now();
    let timed_out = LarkAdapter::new(AppConfig::load(&f.paths).unwrap(), capture)
        .observe()
        .await
        .unwrap();
    assert_eq!(timed_out.state, MeetingState::Unknown);
    assert!(start.elapsed() < Duration::from_secs(10));
    let pid: i32 = fs::read_to_string(f._temp.path().join("observer-pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
}

#[test]
fn review_r1_02_old_observation_cannot_overwrite_manual_control_or_changed_configuration() {
    let f = Fixture::new();
    configure_lark(&f, false);
    let executable = f._temp.path().join("fake-lark");
    let script = fs::read_to_string(&executable).unwrap().replace(
        "if [ false = true ]; then touch \"$root/observer-started\"; exec /bin/sleep 20; fi",
        "touch \"$root/observer-started\"; /bin/sleep 1;",
    );
    fs::write(executable, script).unwrap();
    let mut service = f.daemon();
    await_condition(|| f._temp.path().join("observer-started").exists());
    f.ok(&["record", "start"]);
    thread::sleep(Duration::from_millis(1200));
    assert!(
        f.ok(&["service", "status"])["lastMeetingObservation"].is_null(),
        "pre-control result was applied"
    );
    f.ok(&["record", "auto", "disable"]);
    thread::sleep(Duration::from_millis(1200));
    assert!(
        f.ok(&["service", "status"])["lastMeetingObservation"].is_null(),
        "old configuration result was applied"
    );
    assert_eq!(f.state.lock().unwrap().starts, 1);
    f.ok(&["record", "stop"]);
    f.ok(&["service", "stop"]);
    assert!(service.0.wait().unwrap().success());
}

// Real Swift's synthetic IPC mode, never its hardware recording entry point.
fn prestarted_native_fixture() -> (Fixture, ChildGuard) {
    let mut f = Fixture::new();
    f.proxy.lock().unwrap().take();
    f.stop.store(true, Ordering::SeqCst);
    f.helper.take().unwrap().join().unwrap();
    let socket = f.runtime.join("omm/capture/audio_daemon.sock");
    if socket.exists() {
        fs::remove_file(&socket).unwrap();
    }
    for dir in [
        f.paths.state.join("capture"),
        f.paths.state.join("logs"),
        f.paths.data.join("meetings"),
    ] {
        private_dir(&dir).unwrap();
    }
    let binary = std::env::var_os("OMM_CAPTURE_TEST_BINARY")
        .expect("explicit Swift fixture binary required");
    let child = ChildGuard(
        Command::new(binary)
            .arg("--ipc-fixture")
            .env_clear()
            .env(
                "YULU_APPLICATION_SUPPORT_DIR",
                f.paths.state.join("capture"),
            )
            .env("YULU_LOG_DIR", f.paths.state.join("logs"))
            .env("YULU_MEDIA_LIBRARY_DIR", f.paths.data.join("meetings"))
            .env("YULU_IPC_DIR", f.runtime.join("omm/capture"))
            .env("YULU_SERVICE_OWNER", f.instance.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    await_condition(|| socket.exists());
    (f, child)
}
fn wait_success(child: &mut ChildGuard) {
    let mut exit = None;
    await_condition(|| {
        exit = child.0.try_wait().unwrap();
        exit.is_some()
    });
    assert!(exit.unwrap().success());
}
#[test]
#[ignore = "requires explicit Swift synthetic fixture binary; no hardware"]
fn review_r1_03_adopted_swift_normal_stop_seals_and_verifies_process_exit() {
    let (f, mut helper) = prestarted_native_fixture();
    let mut service = f.daemon();
    f.ok(&["record", "start", "--title", "Synthetic adoption"]);
    let sealed = f.ok(&["service", "stop"]);
    assert!(sealed["meetingId"].is_string());
    wait_success(&mut service);
    assert!(
        helper.0.try_wait().unwrap().is_some(),
        "service stop reported success while adopted Swift helper remained alive"
    );
    wait_success(&mut helper);
    assert!(!f.runtime.join("omm/capture/audio_daemon.sock").exists());
    let store = Store::open(&f.paths).unwrap();
    assert_eq!(store.meetings().unwrap().len(), 1);
    assert!(store.meetings().unwrap()[0].segments[0].path.exists());
}

#[test]
fn core_capture_helper_proxy_survives_disconnected_requests_before_service_restart() {
    let f = Fixture::new();
    let mut service = f.daemon();
    // Hold the broker so the client is gone before the proxy can send its reply.
    {
        let _state = f.state.lock().unwrap();
        let mut stream =
            UnixStream::connect(f.runtime.join("omm/capture/audio_daemon.sock")).unwrap();
        stream.write_all(br#"{"action":"status"}"#).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
    }
    f.ok(&["record", "status"]);
    for interrupted in [b"".as_slice(), b"{\"action\":".as_slice()] {
        let mut stream =
            UnixStream::connect(f.runtime.join("omm/capture/audio_daemon.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream.write_all(interrupted).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        assert!(
            response.is_empty(),
            "interrupted request must not be acknowledged"
        );
        f.ok(&["record", "status"]);
        assert!(
            proxy_alive(&f),
            "disconnected client terminated helper fixture"
        );
    }
    // Keep a truncated request open, as an interrupted client with a live
    // duplicate endpoint would. It must not block the next valid request.
    let mut stalled = UnixStream::connect(f.runtime.join("omm/capture/audio_daemon.sock")).unwrap();
    stalled
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stalled.write_all(b"{\"action\":").unwrap();
    f.ok(&["record", "status"]);
    let mut discarded = Vec::new();
    stalled.read_to_end(&mut discarded).unwrap();
    assert!(discarded.is_empty());
    assert!(proxy_alive(&f));
    service.0.kill().unwrap();
    service.0.wait().unwrap();
    let mut restarted = f.daemon();
    f.ok(&["record", "status"]);
    f.ok(&["service", "stop"]);
    wait_success(&mut restarted);
    assert!(!proxy_alive(&f));
}

#[tokio::test]
async fn core_capture_broker_reply_reaches_eof_while_a_duplicate_endpoint_is_held() {
    use omm::adapters::capture::{CapturePaths, MacCaptureAdapter};
    let f = Fixture::new();
    f.state.lock().unwrap().retain_reply_endpoint = true;
    for path in [
        f.paths.state.join("capture"),
        f.paths.state.join("logs"),
        f.paths.data.join("meetings"),
    ] {
        private_dir(&path).unwrap();
    }
    let capture = MacCaptureAdapter::new(
        CapturePaths {
            state: f.paths.state.join("capture"),
            logs: f.paths.state.join("logs"),
            media: f.paths.data.join("meetings"),
            ipc: f.runtime.join("omm/capture"),
        },
        f.instance,
        Duration::from_secs(2),
    )
    .unwrap();
    assert!(!capture.status().await.unwrap().recording);
    assert!(f.state.lock().unwrap().reply_endpoint.is_some());
    capture.shutdown().await.unwrap();
}

fn read_capture_fixture_request(stream: &mut UnixStream) -> std::io::Result<Vec<u8>> {
    use std::os::fd::AsRawFd;
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::from(std::io::ErrorKind::TimedOut));
        }
        let mut ready = libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut ready, 1, remaining.as_millis().max(1) as i32) };
        if result == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::TimedOut));
        }
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}

// An independent process is essential: the adapter verifies a kernel exit event,
// not merely a reply from a thread in the test runner. The broker retains the
// existing recording-state assertions; this proxy owns only IPC and quit.
#[test]
#[ignore = "subprocess-only core capture fixture"]
fn core_capture_helper_proxy() {
    let Some(socket) = std::env::var_os("OMM_TEST_CAPTURE_SOCKET") else {
        return;
    };
    let socket = PathBuf::from(socket);
    let broker = std::env::var_os("OMM_TEST_CAPTURE_BROKER").unwrap();
    let instance = std::env::var("OMM_TEST_CAPTURE_INSTANCE").unwrap();
    let listener = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    for accepted in listener.incoming() {
        let mut stream = accepted.unwrap();
        // Like Swift's prepareClient, bound half-open clients; other fixture
        // requests have a two-second budget, so release this queue within one.
        // SIGKILL can interrupt a request or close its reply reader. Neither
        // should terminate this independently owned helper (unlike broker errors).
        let mut bytes = match read_capture_fixture_request(&mut stream) {
            Ok(bytes) => bytes,
            Err(error) => {
                assert!(
                    matches!(
                        error.kind(),
                        std::io::ErrorKind::BrokenPipe
                            | std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::ConnectionAborted
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::WouldBlock
                    ),
                    "{error}"
                );
                continue;
            }
        };
        let Ok(request) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let quit = request["action"] == "quit";
        let mut upstream = UnixStream::connect(&broker).unwrap();
        upstream
            .write_all(if quit {
                br#"{"action":"status"}"#
            } else {
                &bytes
            })
            .unwrap();
        upstream.shutdown(std::net::Shutdown::Write).unwrap();
        bytes.clear();
        upstream.read_to_end(&mut bytes).unwrap();
        let mut response: Value = serde_json::from_slice(&bytes).unwrap();
        if quit {
            assert_eq!(request["instance_id"], instance);
            assert_eq!(request["captureIPCVersion"], 1);
            assert_eq!(response["recording"], false, "quit before sealing");
            if response["quitError"] == true {
                response = json!({"captureIPCVersion":1,"instance_id":instance,"status":"refused"});
            } else {
                response = json!({"captureIPCVersion":1,"instance_id":instance,"status":"bye"});
            }
        }
        response["pid"] = json!(std::process::id());
        if let Err(error) = stream.write_all(&serde_json::to_vec(&response).unwrap()) {
            assert!(
                matches!(
                    error.kind(),
                    std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                ),
                "{error}"
            );
        }
        drop(stream);
        if quit && response["status"] == "bye" {
            drop(listener);
            fs::remove_file(socket).unwrap();
            return;
        }
    }
}

#[derive(Default)]
struct RegistrationFixture(Mutex<Vec<String>>);
#[async_trait::async_trait]
impl omm::service::registration::ServiceRunner for RegistrationFixture {
    async fn run(&self, args: &[String]) -> omm::core::contracts::Result<()> {
        self.0.lock().unwrap().push(args[0].clone());
        Ok(())
    }
}
fn fixture_home(f: &Fixture) -> PathBuf {
    f.paths
        .config
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}
fn fixture_runtime(f: &Fixture) -> omm::core::runtime::RuntimeLocation {
    omm::core::runtime::RuntimeLocation::resolve(
        &std::collections::BTreeMap::from([(
            "XDG_RUNTIME_DIR".into(),
            f.runtime.to_str().unwrap().into(),
        )]),
        || panic!("explicit isolated runtime required"),
    )
    .unwrap()
}
fn install_fixture_profile(f: &Fixture, runner: &RegistrationFixture) {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(omm::service::registration::install_at(
            &fixture_home(f),
            &f.paths,
            &std::env::current_exe().unwrap(),
            Some(&fixture_runtime(f)),
            runner,
        ))
        .unwrap();
}
fn uninstall_fixture(
    f: &Fixture,
    runner: &RegistrationFixture,
) -> omm::core::contracts::Result<Value> {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(omm::service::registration::uninstall_at(
            &fixture_home(f),
            &f.paths,
            &fixture_runtime(f),
            runner,
        ))
}
fn proxy_alive(f: &Fixture) -> bool {
    f.proxy
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .0
        .try_wait()
        .unwrap()
        .is_none()
}
#[test]
fn review_r1_03_stop_and_uninstall_reject_wrong_identity_busy_and_failed_quit() {
    for mode in ["wrong_identity", "busy", "failed_quit"] {
        let f = Fixture::new();
        let runner = RegistrationFixture::default();
        install_fixture_profile(&f, &runner);
        let mut service = f.daemon();
        {
            let mut state = f.state.lock().unwrap();
            match mode {
                "wrong_identity" => state.wrong_identity = true,
                "busy" => state.recording = true,
                "failed_quit" => state.quit_error = true,
                _ => unreachable!(),
            }
        }
        let expected = if mode == "failed_quit" {
            ErrorCode::Corrupt
        } else {
            ErrorCode::Conflict
        };
        let rejected = f.invoke(&["service", "stop"]);
        assert!(!rejected.status.success());
        let error: Value = serde_json::from_slice(&rejected.stdout).unwrap();
        assert_eq!(
            error["error"]["code"],
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(uninstall_fixture(&f, &runner).unwrap_err().code, expected);
        assert!(proxy_alive(&f), "rejected helper must not be killed");
        assert!(service.0.try_wait().unwrap().is_none());
        assert_eq!(*runner.0.lock().unwrap(), ["bootstrap"]);
        assert!(omm::service::registration::installed(&fixture_home(&f))
            .unwrap()
            .is_some());
        {
            let mut state = f.state.lock().unwrap();
            state.wrong_identity = false;
            state.recording = false;
            state.quit_error = false;
        }
        f.ok(&["service", "stop"]);
        wait_success(&mut service);
        assert!(!proxy_alive(&f));
    }
}
#[test]
fn review_r1_03_missing_adopted_endpoint_is_not_success_but_never_started_is() {
    let mut f = Fixture::new();
    let mut service = f.daemon();
    f.proxy.lock().unwrap().take();
    fs::remove_file(f.runtime.join("omm/capture/audio_daemon.sock")).unwrap();
    assert_eq!(f.error(&["service", "stop"], 3)["code"], "unavailable");
    assert!(service.0.try_wait().unwrap().is_none());
    // The persisted expectation survives a service restart; absence isn't exit proof.
    service.0.kill().unwrap();
    service.0.wait().unwrap();
    f.stop.store(true, Ordering::SeqCst);
    f.helper.take().unwrap().join().unwrap(); // no automatic proxy restart
    let mut restarted = f.daemon();
    assert_eq!(f.error(&["service", "stop"], 3)["code"], "unavailable");
    restarted.0.kill().unwrap();
    restarted.0.wait().unwrap();
    Store::open(&f.paths)
        .unwrap()
        .clear_state("capture_helper_expected")
        .unwrap();
    let mut never_started = f.daemon();
    f.ok(&["service", "stop"]);
    wait_success(&mut never_started);
}
#[test]
fn review_r1_03_seal_failure_retains_recording_and_helper_for_reconciliation() {
    let f = Fixture::new();
    let _service = f.daemon();
    f.ok(&["record", "start"]);
    let file = f.state.lock().unwrap().file.clone().unwrap();
    fs::write(&file, b"invalid synthetic audio").unwrap();
    assert!(!f.invoke(&["service", "stop"]).status.success());
    assert!(proxy_alive(&f));
    assert!(file.exists());
    assert!(Store::open(&f.paths)
        .unwrap()
        .state::<omm::service::daemon::Recording>("recording")
        .unwrap()
        .is_some());
}
#[test]
#[ignore = "requires explicit Swift synthetic fixture binary; no hardware"]
fn review_r1_03_adopted_swift_uninstall_seals_exits_then_unregisters_without_data_loss() {
    let (f, mut helper) = prestarted_native_fixture();
    let runner = RegistrationFixture::default();
    install_fixture_profile(&f, &runner);
    let mut service = f.daemon();
    f.ok(&["record", "start", "--title", "Synthetic uninstall"]);
    let result = uninstall_fixture(&f, &runner).unwrap();
    assert!(result["sealed"]["meetingId"].is_string());
    wait_success(&mut service);
    assert!(
        helper.0.try_wait().unwrap().is_some(),
        "uninstall succeeded before helper exit"
    );
    wait_success(&mut helper);
    assert!(!f.runtime.join("omm/capture/audio_daemon.sock").exists());
    assert_eq!(*runner.0.lock().unwrap(), ["bootstrap", "bootout"]);
    assert!(omm::service::registration::installed(&fixture_home(&f))
        .unwrap()
        .is_none());
    let store = Store::open(&f.paths).unwrap();
    assert_eq!(store.meetings().unwrap().len(), 1);
    assert!(store.meetings().unwrap()[0].segments[0].path.exists());
    assert!(f.paths.state.exists());
}

#[test]
fn review_r1_03_inactive_uninstall_preserves_profile_until_expected_helper_exit_is_reconciled() {
    let mut f = Fixture::new();
    f.proxy.lock().unwrap().take();
    f.stop.store(true, Ordering::SeqCst);
    f.helper.take().unwrap().join().unwrap();
    fs::remove_file(f.runtime.join("omm/capture/audio_daemon.sock")).unwrap();
    let runner = RegistrationFixture::default();
    install_fixture_profile(&f, &runner);
    Store::open(&f.paths)
        .unwrap()
        .set_state("capture_helper_expected", &true)
        .unwrap();
    assert_eq!(
        uninstall_fixture(&f, &runner).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(*runner.0.lock().unwrap(), ["bootstrap"]);
    Store::open(&f.paths)
        .unwrap()
        .clear_state("capture_helper_expected")
        .unwrap();
    assert_eq!(
        uninstall_fixture(&f, &runner).unwrap()["unregistered"],
        true
    );
    assert_eq!(*runner.0.lock().unwrap(), ["bootstrap", "bootout"]);
    assert!(f.paths.data.exists());
}
#[test]
#[ignore = "requires explicit Swift synthetic fixture binary; no hardware"]
fn review_r1_03_owned_swift_child_is_reaped_and_later_service_needs_no_helper() {
    let mut f = Fixture::new();
    f.proxy.lock().unwrap().take();
    f.stop.store(true, Ordering::SeqCst);
    f.helper.take().unwrap().join().unwrap();
    fs::remove_file(f.runtime.join("omm/capture/audio_daemon.sock")).unwrap();
    let binary = PathBuf::from(
        std::env::var_os("OMM_CAPTURE_TEST_BINARY")
            .expect("explicit Swift fixture binary required"),
    );
    let wrapper = f._temp.path().join("fixture-only-capture");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexec '{}' --ipc-fixture\n",
            binary.to_str().unwrap().replace('\'', "'\\''")
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let mut cfg = AppConfig::load(&f.paths).unwrap();
    cfg.capture_executable = wrapper;
    cfg.save(&f.paths).unwrap();
    let mut service = f.daemon();
    f.ok(&["record", "start"]);
    f.ok(&["service", "stop"]);
    wait_success(&mut service);
    assert!(!f.runtime.join("omm/capture/audio_daemon.sock").exists());
    assert!(Store::open(&f.paths)
        .unwrap()
        .state::<bool>("capture_helper_expected")
        .unwrap()
        .is_none());
    let mut restarted = f.daemon();
    f.ok(&["service", "stop"]);
    wait_success(&mut restarted);
}

fn unstarted_recording_fixture() -> (
    Fixture,
    omm::service::daemon::Controller,
    omm::service::daemon::Recording,
) {
    use omm::adapters::capture::{CapturePaths, MacCaptureAdapter};
    use omm::service::daemon::{Controller, Recording};
    let f = Fixture::new();
    for path in [
        f.paths.state.join("capture"),
        f.paths.state.join("logs"),
        f.paths.data.join("meetings"),
    ] {
        private_dir(&path).unwrap();
    }
    let capture = Arc::new(
        MacCaptureAdapter::new(
            CapturePaths {
                state: f.paths.state.join("capture"),
                logs: f.paths.state.join("logs"),
                media: f.paths.data.join("meetings"),
                ipc: f.runtime.join("omm/capture"),
            },
            f.instance,
            Duration::from_secs(2),
        )
        .unwrap(),
    );
    let cfg = AppConfig::load(&f.paths).unwrap();
    let id = Uuid::new_v4();
    let directory = f.paths.data.join("meetings").join(id.to_string());
    private_dir(&directory).unwrap();
    let record = Recording {
        session_id: id,
        title: "Synthetic unstarted intent".into(),
        start_confirmed: false,
        directory,
        source_id: None,
        started_at_ms: 1,
        segment_started_ms: 1,
        segment_index: 0,
        config: cfg.snapshot(),
        gaps: vec![],
        segments: vec![],
        rotation_pending: false,
        sealed: None,
    };
    Store::open(&f.paths)
        .unwrap()
        .set_state("recording", &record)
        .unwrap();
    let controller = Controller::new(f.paths.clone(), capture, f.instance, &cfg);
    (f, controller, record)
}

#[tokio::test]
async fn unstarted_recording_reconcile_and_stop_clear_only_empty_idle_intent() {
    let (f, mut controller, record) = unstarted_recording_fixture();
    let mut recovery_record = record.clone();
    recovery_record.rotation_pending = true;
    recovery_record.gaps.push("helper_restart".into());
    Store::open(&f.paths)
        .unwrap()
        .set_state("recording", &recovery_record)
        .unwrap();
    // Controller reads the persisted intent, just as a restarted service does.
    controller.reconcile().await.unwrap();
    assert!(controller.recording().unwrap().is_none());
    assert!(Store::open(&f.paths).unwrap().tasks().unwrap().is_empty());
    Store::open(&f.paths)
        .unwrap()
        .set_state("recording", &record)
        .unwrap();
    let reply = controller.seal(true).await.unwrap();
    assert_eq!(reply["unstartedSessionCleared"], true);
    assert!(controller.recording().unwrap().is_none());
    controller
        .start("Next controlled session".into(), None)
        .await
        .unwrap();
    assert_eq!(f.state.lock().unwrap().starts, 1);
    controller.seal(true).await.unwrap();
}

#[tokio::test]
async fn unstarted_recording_keeps_partial_audio_other_identity_and_confirmed_session() {
    let (f, mut controller, mut record) = unstarted_recording_fixture();
    let partial = record.directory.join("segment-0000.partial");
    fs::write(&partial, b"partial synthetic audio").unwrap();
    assert_eq!(
        controller.reconcile().await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        controller.seal(true).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(partial.exists());
    assert!(controller.recording().unwrap().is_some());
    fs::remove_file(partial).unwrap();
    f.state.lock().unwrap().session = Some(Uuid::new_v4());
    assert_eq!(
        controller.reconcile().await.unwrap_err().code,
        ErrorCode::Conflict
    );
    f.state.lock().unwrap().session = None;
    record.start_confirmed = true;
    record.rotation_pending = true;
    Store::open(&f.paths)
        .unwrap()
        .set_state("recording", &record)
        .unwrap();
    assert_eq!(
        controller.reconcile().await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(controller.recording().unwrap().unwrap().rotation_pending);
    assert!(Store::open(&f.paths).unwrap().tasks().unwrap().is_empty());
}

#[tokio::test]
async fn unstarted_recording_keeps_unknown_helper_path_and_legacy_intent() {
    let (f, mut controller, record) = unstarted_recording_fixture();
    f.state.lock().unwrap().wrong_identity = true;
    assert!(controller.reconcile().await.is_err());
    assert!(controller.recording().unwrap().is_some());
    f.state.lock().unwrap().wrong_identity = false;
    f.state.lock().unwrap().file = Some(record.directory.join("not-yet-created.wav"));
    assert!(controller.reconcile().await.is_err());
    assert!(controller.recording().unwrap().is_some());
    f.state.lock().unwrap().file = None;
    let mut legacy = serde_json::to_value(&record).unwrap();
    legacy.as_object_mut().unwrap().remove("start_confirmed");
    Store::open(&f.paths)
        .unwrap()
        .set_state("recording", &legacy)
        .unwrap();
    assert!(controller.recording().unwrap().unwrap().start_confirmed);
    assert_eq!(
        controller.reconcile().await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(controller.recording().unwrap().is_some());
}

#[tokio::test]
async fn unstarted_recording_late_success_is_confirmed_before_new_start() {
    let (f, mut controller, record) = unstarted_recording_fixture();
    // Lost start reply, followed by an authenticated observation of that session.
    let wav_path = record.directory.join("segment-0000.wav");
    let mut wav = hound::WavWriter::create(
        &wav_path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..9600 {
        wav.write_sample(0i16).unwrap();
    }
    wav.finalize().unwrap();
    std::fs::set_permissions(&wav_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    {
        let mut state = f.state.lock().unwrap();
        state.recording = true;
        state.session = Some(record.session_id);
        state.file = Some(record.directory.join("segment-0000.wav"));
    }
    assert_eq!(
        controller
            .start("Must not overlap".into(), None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    controller.reconcile().await.unwrap();
    assert!(controller.recording().unwrap().unwrap().start_confirmed);
    assert_eq!(f.state.lock().unwrap().starts, 0);
    // Even if the helper then reports idle without an identity, confirmed intent
    // cannot be erased merely because an audio file has not appeared yet.
    *f.state.lock().unwrap() = HelperState::default();
    fs::remove_file(wav_path).unwrap();
    assert_eq!(
        controller.reconcile().await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(controller.recording().unwrap().is_some());
}

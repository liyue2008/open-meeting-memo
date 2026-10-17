//! Identity-bound synthetic capture peer; compiled only with the private native fixture feature.
use omm::core::{
    paths::*,
    store::{hash_file, Store},
};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};
use uuid::Uuid;
pub struct ChildGuard(pub Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
pub fn start(paths: &Paths) -> ChildGuard {
    let instance = Store::open_read_only(paths)
        .unwrap()
        .state::<Uuid>("capture_instance")
        .unwrap()
        .unwrap();
    let socket = socket(paths);
    private_dir(socket.parent().unwrap()).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .env("OMM_NATIVE_FIXTURE_HELPER", "1")
        .env("OMM_NATIVE_FIXTURE_CAPTURE_INSTANCE", instance.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut guard = ChildGuard(child);
    for _ in 0..200 {
        if socket.exists() {
            return guard;
        }
        assert!(
            guard.0.try_wait().unwrap().is_none(),
            "synthetic capture peer exited"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("synthetic capture peer did not bind");
}
fn socket(paths: &Paths) -> PathBuf {
    paths
        .runtime_location(&std::env::vars().collect())
        .unwrap()
        .path
        .join("capture/audio_daemon.sock")
}
pub fn run() {
    let root = super::output("driver.js");
    validate_private_file(&root).unwrap();
    let paths = Paths::from_env().unwrap();
    assert!(paths.data.starts_with(root.parent().unwrap()));
    let instance =
        Uuid::parse_str(&std::env::var("OMM_NATIVE_FIXTURE_CAPTURE_INSTANCE").unwrap()).unwrap();
    let socket = socket(&paths);
    let listener = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut session = None::<Uuid>;
    let mut file = None::<PathBuf>;
    let mut recording = false;
    for accepted in listener.incoming() {
        let mut stream = accepted.unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = vec![];
        if Read::by_ref(&mut stream)
            .take(1024 * 1024)
            .read_to_end(&mut bytes)
            .is_err()
        {
            continue;
        }
        let Ok(req) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let mut reply = match req["action"].as_str().unwrap_or("") {
            "status" => {
                json!({"captureIPCVersion":1,"instance_id":instance,"recording":recording,"session_id":session.map(|v|v.to_string()).unwrap_or_default(),"file":if recording {file.as_ref().and_then(|p|p.to_str()).unwrap_or("")} else {""},"sysReady":true,"micReady":true})
            }
            "start" => {
                let id = Uuid::parse_str(req["session_id"].as_str().unwrap()).unwrap();
                let dir = PathBuf::from(req["output_dir"].as_str().unwrap());
                assert!(dir.starts_with(paths.data.join("meetings")));
                let name = req["filename"].as_str().unwrap();
                assert!(!name.contains('/'));
                let output = dir.join(name);
                let mut wav = omm::application::media::silent_wav(6);
                wav[22..24].copy_from_slice(&2u16.to_le_bytes());
                wav[24..28].copy_from_slice(&48000u32.to_le_bytes());
                wav[28..32].copy_from_slice(&192000u32.to_le_bytes());
                wav[32..34].copy_from_slice(&4u16.to_le_bytes());
                write_private_new(&output, &wav).unwrap();
                file = Some(output.clone());
                session = Some(id);
                recording = true;
                json!({"instance_id":instance,"session_id":id,"status":"recording","file":output})
            }
            "stop" => {
                recording = false;
                json!({"instance_id":instance,"session_id":session,"status":"stopped","paged_segments":true,"file":file,"duration":1.0})
            }
            "sealed_segments" => {
                let mut rows = vec![];
                if !recording && req["after_sequence"].as_i64().unwrap_or(-1) < 0 {
                    if let Some(path) = &file {
                        let document = json!({"schema_version":1,"session_id":session,"instance_id":instance,"sequence":0,"offset_frames":0,"frame_count":48000,"sample_rate":48000,"path":path,"temporary":false,"legacy_wav":false});
                        let descriptor = PathBuf::from(format!("{}.sealed.json", path.display()));
                        if !descriptor.exists() {
                            write_private_new(&descriptor, &serde_json::to_vec(&document).unwrap())
                                .unwrap()
                        }
                        rows.push(json!({"document":document,"descriptor_sha256":hash_file(&descriptor).unwrap(),"sha256":hash_file(path).unwrap()}));
                    }
                }
                json!({"instance_id":instance,"origin_instance_id":instance,"session_id":session,"segments":rows,"has_more":false})
            }
            "windows" => json!({"windows":[]}),
            "audio_devices" => json!({"input":[],"default_input_uid":""}),
            "quit" => {
                assert!(!recording);
                json!({"captureIPCVersion":1,"instance_id":instance,"status":"bye"})
            }
            _ => json!({"error":"unknown synthetic operation"}),
        };
        reply["pid"] = json!(std::process::id());
        let _ = stream.write_all(&serde_json::to_vec(&reply).unwrap());
        let _ = stream.shutdown(std::net::Shutdown::Write);
        if req["action"] == "quit" {
            break;
        }
    }
    drop(listener);
    let _ = std::fs::remove_file(socket);
}

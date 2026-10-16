use super::*;
use std::process::Command;

#[test]
fn c_api_fixture_decode_and_every_owned_handle_is_released_on_failure() {
    let temp = tempfile::tempdir().unwrap();
    let lib = temp.path().join(if cfg!(target_os = "macos") {
        "fixture.dylib"
    } else {
        "fixture.so"
    });
    let status = Command::new("cc")
        .args([
            "-shared",
            "-fPIC",
            "-std=c11",
            "-Wall",
            "-Werror",
            "tests/asr_fixtures/c_api_fixture.c",
            "-o",
        ])
        .arg(&lib)
        .status()
        .unwrap();
    assert!(status.success());
    let probe = unsafe { Library::new(&lib).unwrap() };
    let set_mode = unsafe {
        probe
            .get::<unsafe extern "C" fn(i32)>(b"FixtureMode\0")
            .unwrap()
    };
    let count = unsafe {
        probe
            .get::<unsafe extern "C" fn(i32) -> i32>(b"FixtureActive\0")
            .unwrap()
    };
    let decodes = unsafe {
        probe
            .get::<unsafe extern "C" fn() -> i32>(b"FixtureDecodes\0")
            .unwrap()
    };
    let last_input = unsafe {
        probe
            .get::<unsafe extern "C" fn() -> i32>(b"FixtureLastInput\0")
            .unwrap()
    };
    let audio = temp.path().join("synthetic.f32");
    std::fs::write(
        &audio,
        (0..RATE * 8)
            .flat_map(|_| 0.1f32.to_le_bytes())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let cfg = AsrConfig {
        library_path: lib,
        encoder_path: "/fixture/encoder".into(),
        decoder_path: "/fixture/decoder".into(),
        tokens_path: "/fixture/tokens".into(),
        vad_path: "/fixture/vad".into(),
        threads: 2,
        ..Default::default()
    };
    for mode in 0..=6 {
        unsafe {
            set_mode(mode);
        }
        let engine = NativeEngine::new(&cfg);
        if mode == 5 || mode == 6 {
            assert!(engine.is_err());
        } else {
            let engine = engine.unwrap();
            let result = engine.transcribe_file(&audio, RATE, AudioSource::Mic, 1000);
            if mode == 0 {
                let segments = result.unwrap();
                assert_eq!(segments.len(), 1);
                assert_eq!((segments[0].start_ms, segments[0].end_ms), (1100, 1300));
                assert_eq!(segments[0].text, "测试 meeting");
                assert_eq!(unsafe { decodes() }, 1);
                assert_eq!(unsafe { last_input() }, 8000);
            } else {
                assert!(result.is_err(), "fixture mode {mode}");
            }
            drop(engine);
        }
        for i in 0..5 {
            assert_eq!(unsafe { count(i) }, 0, "mode {mode}, handle {i}");
        }
    }
    unsafe {
        set_mode(7);
    }
    let before = unsafe { decodes() };
    let engine = NativeEngine::new(&cfg).unwrap();
    let result = engine
        .transcribe_file(&audio, RATE * 8, AudioSource::System, 0)
        .unwrap();
    assert_eq!(result.len(), 10);
    assert_eq!(unsafe { decodes() } - before, 10); // FireRed's decode-streams is serial: publish each span immediately.
    assert!(result.windows(2).all(|s| s[0].start_ms < s[1].start_ms));
    unsafe {
        set_mode(9);
    }
    let crossing = engine
        .transcribe_file(&audio, RATE * 8, AudioSource::Mic, 0)
        .unwrap();
    assert_eq!(crossing.len(), 1);
    assert_eq!((crossing[0].start_ms, crossing[0].end_ms), (2044, 2450));
    assert_eq!(unsafe { last_input() }, 12900);
    drop(engine);
    for i in 0..5 {
        assert_eq!(unsafe { count(i) }, 0);
    }
}

#[test]
fn native_oversized_vad_spans_are_split_before_decode_including_padding() {
 let temp=tempfile::tempdir().unwrap();let lib=temp.path().join("bounded.dylib");
 assert!(Command::new("cc").args(["-shared","-fPIC","-std=c11","tests/asr_fixtures/c_api_fixture.c","-o"]).arg(&lib).status().unwrap().success());
 let probe=unsafe{Library::new(&lib).unwrap()};
 let mode=unsafe{probe.get::<unsafe extern "C" fn(i32)>(b"FixtureMode\0").unwrap()};
 let maximum=unsafe{probe.get::<unsafe extern "C" fn()->i32>(b"FixtureMaxInput\0").unwrap()};
 let audio=temp.path().join("generated.f32");
 std::fs::write(&audio,(0..RATE*60).flat_map(|_|0.1f32.to_le_bytes()).collect::<Vec<_>>()).unwrap();
 let cfg=AsrConfig{library_path:lib,threads:2,..Default::default()};
 for (m,end) in [(10,27382),(11,60000)] {
  unsafe{mode(m)};let engine=NativeEngine::new(&cfg).unwrap();
  let segments=engine.transcribe_file(&audio,RATE*60,AudioSource::Mic,1000).unwrap();
  assert!(unsafe{maximum()}<=400000,"actual padded decoder input exceeds 25 seconds");
  assert_eq!(segments.first().unwrap().start_ms,1000);
  assert_eq!(segments.last().unwrap().end_ms,1000+end);
  assert!(segments.windows(2).all(|s|s[0].end_ms==s[1].start_ms));
 }
}

#[test]
fn native_timeout_checkpoint_survives_queue_cleanup_restart_and_reuses_only_completed_chunk() {
 use crate::core::{paths::{Paths,private_dir,write_private_new},store::{Store,hash_file}};
 let t=tempfile::tempdir().unwrap();let root=t.path().canonicalize().unwrap();
 let paths=Paths::resolve(&root.join("home"),&std::collections::BTreeMap::new()).unwrap();paths.create_private_dirs().unwrap();
 let lib=root.join("retry.dylib");assert!(Command::new("cc").args(["-shared","-fPIC","-std=c11","tests/asr_fixtures/c_api_fixture.c","-o"]).arg(&lib).status().unwrap().success());
 let probe=unsafe{Library::new(&lib).unwrap()};let mode=unsafe{probe.get::<unsafe extern "C" fn(i32)>(b"FixtureMode\0").unwrap()};let decodes=unsafe{probe.get::<unsafe extern "C" fn()->i32>(b"FixtureDecodes\0").unwrap()};
 unsafe{mode(11)};
 let session=uuid::Uuid::new_v4();let meeting_dir=paths.data.join("meetings").join(session.to_string());private_dir(&meeting_dir).unwrap();let audio=meeting_dir.join("generated.float");write_private_new(&audio,&(0..RATE*60).flat_map(|_|0.1f32.to_le_bytes()).collect::<Vec<_>>()).unwrap();
 let cfg=AsrConfig{library_path:lib,threads:2,..Default::default()};let meeting=SealedMeeting{session_id:session,title:"generated".into(),directory:meeting_dir,segments:vec![AudioSegment{capture:None,path:audio.clone(),duration_ms:60000,sha256:hash_file(&audio).unwrap()}],sealed_at_ms:1};
 let mut store=Store::open(&paths).unwrap();let task=store.enqueue(&meeting,&ConfigSnapshot{asr:cfg.clone(),..Default::default()},false).unwrap();let claim=store.claim(10,100).unwrap().unwrap();
 let owner=claim.context.work_dir.parent().unwrap();let checkpoint=owner.join(".chunks");private_dir(&checkpoint).unwrap();let cache=super::super::chunks::directory(&checkpoint,&audio,&cfg,AudioSource::Mic).unwrap();
 let engine=NativeEngine::new(&cfg).unwrap();let error=engine.transcribe_file_progress(&audio,RATE*60,AudioSource::Mic,0,Some(cache.clone()),Box::new(|done,_,_,_|if done==1 {Err(OmmError::new(ErrorCode::Timeout,true,"generated timeout"))}else{Ok(())})).unwrap_err();drop(engine);assert_eq!(unsafe{decodes()},1);
 store.fail(&claim,&error,20).unwrap();store.schedule_retry(&store.task(task.id).unwrap(),20).unwrap();store.clean_abandoned_cache(20).unwrap();assert!(cache.exists());assert!(!claim.context.work_dir.exists());drop(store);
 let mut store=Store::open(&paths).unwrap();store.recover(2000).unwrap();store.clean_abandoned_cache(2000).unwrap();let retry=store.claim(2000,100).unwrap().unwrap();assert_eq!(retry.task.id,task.id);
 let engine=NativeEngine::new(&cfg).unwrap();let before=unsafe{decodes()};let result=engine.transcribe_file_progress(&audio,RATE*60,AudioSource::Mic,0,Some(cache.clone()),Box::new(|_,_,_,_|Ok(()))).unwrap();assert_eq!(result.len(),3);assert_eq!(unsafe{decodes()}-before,2);drop(engine);
 for changed in [AsrConfig{threads:1,..cfg.clone()},AsrConfig{model_version:"changed-model".into(),..cfg.clone()}] {assert_ne!(cache,super::super::chunks::directory(&checkpoint,&audio,&changed,AudioSource::Mic).unwrap());}
 std::fs::write(&audio,(0..RATE*60).flat_map(|_|0.2f32.to_le_bytes()).collect::<Vec<_>>()).unwrap();assert_ne!(cache,super::super::chunks::directory(&checkpoint,&audio,&cfg,AudioSource::Mic).unwrap());
}

#[test]
fn native_measurement_delay_cpu_cache_and_token_counts() {
 let output=Command::new(std::env::current_exe().unwrap()).args(["--exact","adapters::asr::native::tests::native_measurement_clock_fixture_child","--ignored","--nocapture"]).output().unwrap();
 assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
}
#[test]
#[ignore = "Owned isolated clock fixture, invoked only by its source-named wrapper"]
fn native_measurement_clock_fixture_child() {
 use std::{rc::Rc,cell::RefCell};
 let temp=tempfile::tempdir().unwrap();let root=temp.path().canonicalize().unwrap();let lib=root.join("measurement.dylib");
 assert!(Command::new("cc").args(["-shared","-fPIC","-std=c11","tests/asr_fixtures/c_api_fixture.c","-o"]).arg(&lib).status().unwrap().success());
 let probe=unsafe{Library::new(&lib).unwrap()};let mode=unsafe{probe.get::<unsafe extern "C" fn(i32)>(b"FixtureMode\0").unwrap()};let decodes=unsafe{probe.get::<unsafe extern "C" fn()->i32>(b"FixtureDecodes\0").unwrap()};
 let audio=root.join("generated.f32");std::fs::write(&audio,(0..RATE*8).flat_map(|_|0.1f32.to_le_bytes()).collect::<Vec<_>>()).unwrap();
 let cfg=AsrConfig{library_path:lib,threads:2,..Default::default()};let cache_root=root.join("checkpoints");crate::core::paths::private_dir(&cache_root).unwrap();let cache=super::super::chunks::directory(&cache_root,&audio,&cfg,AudioSource::Mic).unwrap();
 let mut cpu=vec![];
 for (m,tokens) in [(12,Some(7)),(13,Some(0)),(14,None),(15,None)] {
  unsafe{mode(m)};let engine=NativeEngine::new(&cfg).unwrap();let values=Rc::new(RefCell::new(vec![]));let sink=values.clone();
  let result=engine.transcribe_file_measured(&audio,RATE*8,AudioSource::Mic,0,if m==12 {Some(cache.clone())}else{None},Box::new(move|_,_,_,_,t|{if let Some(t)=t {sink.borrow_mut().push(t);}Ok(())})).unwrap();
  let v=values.borrow();assert_eq!(v.len(),1);assert_eq!(v[0].token_count,tokens);assert_eq!(v[0].output_chars,10);assert_eq!(v[0].decode_samples,8000);assert!(v[0].valid());
  if m==12 || m==13 {assert!(v[0].native_decode_wall_us>=20000);cpu.push(v[0].process_cpu_us.unwrap());}
  if m==12 {
   let bytes=std::fs::read_dir(&cache).unwrap().map(|e|{let p=e.unwrap().path();(p.clone(),std::fs::read(p).unwrap())}).collect::<Vec<_>>();
   let before=unsafe{decodes()};let hits=Rc::new(RefCell::new(vec![]));let sink=hits.clone();let again=engine.transcribe_file_measured(&audio,RATE*8,AudioSource::Mic,0,Some(cache.clone()),Box::new(move|_,_,_,_,t|{if let Some(t)=t {sink.borrow_mut().push(t);}Ok(())})).unwrap();
   assert_eq!(serde_json::to_vec(&result).unwrap(),serde_json::to_vec(&again).unwrap());assert_eq!(unsafe{decodes()},before);
   let hits=hits.borrow();assert_eq!(hits.len(),1);assert!(hits[0].cache_hit);assert_eq!(hits[0].native_decode_wall_us,0);assert_eq!(hits[0].process_cpu_us,Some(0));assert_eq!(hits[0].checkpoint_write_us,0);assert_eq!(hits[0].token_count,None);
   for (path,old) in bytes {assert_eq!(std::fs::read(path).unwrap(),old);}
  }
 }
 assert!(cpu[1]>cpu[0]+15000,"process CPU distinguishes spin from sleep in owned isolated fixture");
}

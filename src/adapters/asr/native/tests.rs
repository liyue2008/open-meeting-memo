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

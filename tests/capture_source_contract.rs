// Immutable hashes of the cited Yulu source, with only documented naming substitutions undone.
use sha2::{Digest, Sha256};
#[test]
fn capture_audio_kernel_matches_reference_source() {
    let source = include_str!("../native/yulu-capture/audio_daemon.swift")
        .replace("org.openmeetingmemo.", "com.yulu.")
        .replace("OMM-SysTap", "Yulu-SysTap")
        .replace("let fn = requestedFilename ?? ", "let fn = ")
        // Only the outer writer binding/configuration changed. The acquisition,
        // source pairing, PCM normalization, and underlying WAV writer stay byte-identical.
        .replace(", offsetFrames: offsetFrames, title: title)", ", offsetFrames: offsetFrames)")
        .replace("RollingRecordingWriter", "WavWriter")
        .replace("    private var maxSegmentFrames = 1800 * 48000\n", "")
        .replace(", segmentMs: Int = 1800000", "")
        .replace(
            "            maxSegmentFrames = min(1800000, max(60000, segmentMs)) * 48\n",
            "",
        )
        .replace(
            "func start(title: String, sequence: Int = 0, offsetFrames: Int = 0)",
            "func start(title: String)",
        )
        .replace(
            ", maxFrames: maxSegmentFrames,\n                                                sequence: sequence, offsetFrames: offsetFrames",
            "",
        )
        .replace(", maxFrames: maxSegmentFrames", "")
        // Undo only exact lifecycle/bridge additions; PCM, pairing and WAV kernels remain frozen.
        .replace("            if !bridgeError.isEmpty { return nil }\n", "")
        .replace(r#"            guard writeState(recording: true, title: title, path: url.path, sequence: sequence,
                             offsetFrames: offsetFrames, segmentMs: maxSegmentFrames / 48) else {
                isRecordingState = false; return nil
            }"#, "            writeState(recording: true, title: title, path: url.path)")
        .replace(r#"            guard writer?.failed != true, writeState(recording: false) else {
                bridgeError = "capture_recovery_state_failed"; return (p, dur)
            }
            bridgeError = ""
"#, "")
        .replace(r#"            var bridge = readBridge()
            var gaps = bridge["gaps"] as? [[String: Any]] ?? []
            let wallTimeMs = Int(now.timeIntervalSince1970 * 1000)
            let startedWallMs = bridge["started_wall_ms"] as? Int ?? Int((startTime ?? now).timeIntervalSince1970 * 1000)
            gaps.append(["frame_offset": writer?.framePosition ?? 0, "wall_time_ms": wallTimeMs,
                         "wall_offset_ms": max(0, wallTimeMs - startedWallMs), "kind": "microphone_route_restart"])
            bridge["gaps"] = gaps
            if !saveBridge(bridge) { bridgeError = "capture_recovery_state_failed" }
"#, "");
    let blocks = [
        (
            "anchored WAV writer and normalization",
            "final class AnchoredRecordingDirectory",
            "// ─── 状态管理",
            "b2b36d34f773a04aabe2f9408b46cef58877085a3fb6dd4cdb39d7f62b3b2630",
        ),
        (
            "source-separated PCM synchronization and silence monitor",
            "func micSamplesForRecording",
            "final class MeetingMuteMonitor",
            "30cd9a7f1fff5e8273157c3910fd85bb3609aceb3c7dae673f69ddd572a47fc7",
        ),
        (
            "microphone input route/rebuild",
            "    private func audioDeviceID",
            "// ─── SCStream",
            "40e05666fb18ac2a5c918731ac734c58ea7f1a33d2c92cbbcba803466919c454",
        ),
        (
            "ScreenCaptureKit and process tap recovery",
            "// ─── SCStream",
            "// ─── Socket",
            "eaf2fdef1bdd29a49835ed6fa6f566434c484a82b7b2bf06d9de2b08d0f68888",
        ),
    ];
    for (name, start, end, expected) in blocks {
        let a = source.find(start).unwrap();
        let b = source[a..].find(end).unwrap() + a;
        assert_eq!(
            hex::encode(Sha256::digest(&source.as_bytes()[a..b])),
            expected,
            "copied {name} changed"
        );
    }
    let license = include_str!("../native/yulu-capture/LICENSE");
    assert!(
        source.contains(license),
        "complete MIT license must stay in copied source"
    );
}

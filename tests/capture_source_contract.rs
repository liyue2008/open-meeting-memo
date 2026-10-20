// Immutable hashes of the cited Yulu source, with only documented naming substitutions undone.
use sha2::{Digest, Sha256};

// Only the new input-format adapter and its numeric clock observations are outside
// the imported kernel. Keep every original kernel hash below unchanged.
fn without_input_format_boundary(mut source: String) -> String {
    for label in [
        "state",
        "validate-build",
        "listeners-build",
        "lifecycle-test",
        "helpers",
        "context-buffer",
        "teardown",
        "sck-validate",
    ] {
        let start = format!("// BEGIN input-format {label}");
        let end = format!("// END input-format {label}");
        assert_eq!(source.matches(&start).count(), 1);
        assert_eq!(source.matches(&end).count(), 1);
        let a = source[..source.find(&start).unwrap()].rfind('\n').unwrap() + 1;
        let mut b = source.find(&end).unwrap() + end.len() + 1;
        if matches!(label, "lifecycle-test" | "helpers") {
            assert_eq!(source.as_bytes()[b], b'\n');
            b += 1; // separator belonging only to this inserted method block
        }
        source.replace_range(a..b, "");
    }
    source = source
        .replace("[weak self, context] (_, inInputData, _, _, _) in", "[weak self] (_, inInputData, _, _, _) in")
        .replace("self.handleIO(inInputData, context: context)", "self.handleIO(inInputData)")
        .replace("private func handleIO(_ inInputData: UnsafePointer<AudioBufferList>, context: TapIOFormatContext)", "private func handleIO(_ inInputData: UnsafePointer<AudioBufferList>)")
        .replace("if probe { return publishInputFormatReady(context) }", "if probe { return true }")
        .replace("        return publishInputFormatReady(context)\n    }\n\n    /// Read kAudioTapPropertyUID", "        return true\n    }\n\n    /// Read kAudioTapPropertyUID")
        .replace("        if buildTap(probe: false) {\n            lock.lock()", "        if buildTap(probe: false) {\n            lifecycleLock.unlock()\n            lock.lock()")
        .replace("            SYS_READY = true; SYS_ERROR = \"\"\n            lifecycleLock.unlock()\n            log(\"🔊 Sys tap capture started\")", "            SYS_READY = true; SYS_ERROR = \"\"\n            log(\"🔊 Sys tap capture started\")");
    for (start, end, replacement) in [
        (
            "    private let micRateAdapter =",
            "    // Streaming buffers",
            "\n",
        ),
        (
            "            micRateAdapter.reset();",
            "            isRecordingState = true",
            "",
        ),
        (
            "            if let tail = micRateAdapter.finish()",
            "            flushBuffersOnQueue()",
            "",
        ),
        (
            "            guard samples.count % 2 == 0, sourceRate.isFinite,",
            "            self.sysGapMicFallbackLogged",
            "            self.sysBuf.append(contentsOf: samples)\n",
        ),
        (
            "            guard let normalized = self.micRateAdapter.convert",
            "            let gain = self.micGainState",
            "",
        ),
        (
            "        // The process tap's format, not the requested WAV clock,",
            "        let abl = UnsafeMutableAudioBufferListPointer",
            "",
        ),
    ] {
        assert_eq!(
            source.matches(start).count(),
            1,
            "ambiguous input adapter seam"
        );
        let a = source.find(start).unwrap();
        let b = a + source[a..].find(end).unwrap();
        source.replace_range(a..b, replacement);
    }
    source
        .replace(", sourceRate: Double = 48000", "")
        .replace(
            "micSamplesForRecording(normalized,",
            "micSamplesForRecording(samples,",
        )
        .replace(
            "self.recorder.onMicAudio(samples, sourceRate: buf.format.sampleRate)",
            "self.recorder.onMicAudio(samples)",
        )
        .replace(
            "recorder.onSysAudio(int16s, sourceRate: asbd?.mSampleRate ?? 0)",
            "recorder.onSysAudio(int16s)",
        )
        .replace(
            "recorder.onSysAudio(int16s, sourceRate: sourceRate)",
            "recorder.onSysAudio(int16s)",
        )
}

#[test]
fn process_tap_io_uses_generation_snapshot_without_hal_or_control_lock() {
    let source = include_str!("../native/yulu-capture/audio_daemon.swift");
    let start = source.find("    private func handleIO(").unwrap();
    let end = start + source[start..].find("    /// Pitfall 3 recovery").unwrap();
    let callback = &source[start..end];
    for forbidden in [
        "AudioObjectGetPropertyData",
        "readInputFormat(",
        "lifecycleLock",
        "formatControlQueue.sync",
    ] {
        assert!(
            !callback.contains(forbidden),
            "new realtime query/wait: {forbidden}"
        );
    }
    assert!(
        callback.contains("context.isActive")
            && callback.contains("context.format.rate")
            && callback.contains("context.format.accepts(abl)")
    );
    assert!(source.contains("[weak self, context] (_, inInputData"));
    assert!(source.contains("kAudioDevicePropertyStreamFormat"));
    assert!(source.contains("guard let actual = asbd, CapturePCMFormat(actual) != nil"));
}

// Project only reviewed operational diagnostics back to the 39b6 kernel.
// Each full literal must occur once inside its original class, including its
// adjacent control/data statement. No wildcard, new baseline hash or arbitrary
// logging removal is allowed; every other byte reaches the four original hashes.
fn without_operational_diagnostics(mut source: String) -> String {
    for (start, end, edits) in [
        (
            r#"class AudioRecorder {"#,
            r#"final class MeetingMuteMonitor"#,
            &[
                (
                    r#"    private var sysGapMicFallbackLogged = false
    private var systemNonzeroSeen = false
    private var micGainState:"#,
                    r#"    private var sysGapMicFallbackLogged = false
    private var micGainState:"#,
                ),
                (
                    r#"            guard self.isRecordingState else { return }
            logEvent(.micGate, code: state == .muted ? 1 : (state == .unmuted ? 0 : -1))
            switch state {"#,
                    r#"            guard self.isRecordingState else { return }
            switch state {"#,
                ),
                (
                    r#"            sysGapMicFallbackLogged = false
            systemNonzeroSeen = false
            meetingMicStateState = .unknown"#,
                    r#"            sysGapMicFallbackLogged = false
            meetingMicStateState = .unknown"#,
                ),
                (
                    r#"        return syncState {
            if isRecordingState { logEvent(.stopRequested) }
            isRecordingState = false"#,
                    r#"        return syncState {
            isRecordingState = false"#,
                ),
                (
                    r#"            if !self.systemNonzeroSeen && samples.contains(where: { $0 != 0 }) {
                self.systemNonzeroSeen = true
                logEvent(.systemNonzeroSeen)
            }
            self.sysBuf.append(contentsOf: normalized)"#,
                    r#"            self.sysBuf.append(contentsOf: normalized)"#,
                ),
                (
                    r#"            self.autoStopRequested = true
            logEvent(.silenceThresholdReached)
            let action = SYS_DISABLED"#,
                    r#"            self.autoStopRequested = true
            let action = SYS_DISABLED"#,
                ),
            ][..],
        ),
        (
            r#"final class ProcessTapBackend:"#,
            r#"// ─── Socket"#,
            &[
                (
                    r#"        guard tapErr == noErr, tap != 0 else {
            logEvent(.tapCreateError, code: tapErr)
            _lastError"#,
                    r#"        guard tapErr == noErr, tap != 0 else {
            _lastError"#,
                ),
                (
                    r#"        logEvent(.tapCreateSuccess, code: probe ? 1 : 0)
        tapID = tap"#,
                    r#"        tapID = tap"#,
                ),
                (
                    r#"        guard aggErr == noErr, agg != 0 else {
            logEvent(.aggregateCreateError, code: aggErr)
            _lastError"#,
                    r#"        guard aggErr == noErr, agg != 0 else {
            _lastError"#,
                ),
                (
                    r#"        guard procErr == noErr, let p = proc else {
            logEvent(.ioProcCreateError, code: procErr)
            _lastError"#,
                    r#"        guard procErr == noErr, let p = proc else {
            _lastError"#,
                ),
                (
                    r#"        guard startErr == noErr else {
            logEvent(.deviceStartError, code: startErr)
            _lastError"#,
                    r#"        guard startErr == noErr else {
            _lastError"#,
                ),
                (
                    r#"            if tripped {
                logEvent(.zeroRecovery, code: Int32(clamping: currentZeroRun))
                // Rebuild off the realtime IO thread."#,
                    r#"            if tripped {
                log("⚠️ Sys tap delivered \(currentZeroRun) all-zero buffers — teardown+rebuild (Pitfall 3)")
                // Rebuild off the realtime IO thread."#,
                ),
                (
                    r#"            SYS_READY = true; SYS_ERROR = ""
            logEvent(.zeroRecoverySuccess)"#,
                    r#"            SYS_READY = true; SYS_ERROR = ""
            log("🔊 Sys tap rebuilt after zero-buffer recovery")"#,
                ),
                (
                    r#"            SYS_READY = false; SYS_ERROR = _lastError
            logEvent(.zeroRecoveryFailure)"#,
                    r#"            SYS_READY = false; SYS_ERROR = _lastError
            log("Sys tap rebuild failed: \(_lastError)")"#,
                ),
            ][..],
        ),
    ] {
        assert_eq!(source.matches(start).count(), 1);
        assert_eq!(source.matches(end).count(), 1);
        let a = source.find(start).unwrap();
        let b = a + source[a..].find(end).unwrap();
        let mut section = source[a..b].to_owned();
        for (before, after) in edits {
            assert_eq!(
                section.matches(before).count(),
                1,
                "diagnostic location/count changed: {before}"
            );
            section = section.replacen(before, after, 1);
        }
        assert!(
            !section.contains("logEvent("),
            "unreviewed diagnostic in frozen class"
        );
        assert!(
            !section.contains("systemNonzeroSeen"),
            "unreviewed diagnostic state"
        );
        source.replace_range(a..b, &section);
    }
    source
}

#[test]
fn diagnostic_projection_keeps_unrelated_pcm_and_recovery_edits() {
    let source = include_str!("../native/yulu-capture/audio_daemon.swift");
    for (before, after) in [
        ("$0 * gain", "$0 * 0.0"),
        (
            "if tripped {\n                rebuilding = true",
            "if false {\n                rebuilding = true",
        ),
        (
            "let seconds = silenceSecondsState",
            "let seconds = silenceSecondsState + 1",
        ),
    ] {
        assert_eq!(source.matches(before).count(), 1);
        let changed = source.replacen(before, after, 1);
        assert!(std::panic::catch_unwind(|| assert_kernel_matches_reference(&changed)).is_err());
    }
    for changed in [
        source.replacen(
            "logEvent(.systemNonzeroSeen)",
            "logEvent(.systemNonzeroSeen)\n                logEvent(.systemNonzeroSeen)",
            1,
        ),
        source.replacen(
            "private var systemNonzeroSeen = false",
            "private var systemNonzeroSeen = true",
            1,
        ),
    ] {
        assert!(std::panic::catch_unwind(|| without_operational_diagnostics(changed)).is_err());
    }
}

#[test]
fn capture_audio_kernel_matches_reference_source() {
    assert_kernel_matches_reference(include_str!("../native/yulu-capture/audio_daemon.swift"));
}

fn assert_kernel_matches_reference(source: &str) {
    let source = normalized_kernel_source(without_operational_diagnostics(source.to_owned()));
    assert_kernel_blocks(&source);
}

fn normalized_kernel_source(source: String) -> String {
    without_input_format_boundary(source)
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
"#, "")
}

fn assert_kernel_blocks(source: &str) {
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

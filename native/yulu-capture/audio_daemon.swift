/*
MIT License

Copyright (c) 2026 Yulu contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
*/
// OMM perimeter adaptation; see NOTICE.md. Audio algorithms retain the Yulu source.
// Yulu audio_daemon: AVAudioEngine mic + ScreenCaptureKit sys, source-separated stereo WAV.
// 替代 BlackHole + SoX 的方案。
//
// 编译:
//   swiftc -o audio_daemon audio_daemon.swift \
//     -framework Cocoa -framework ScreenCaptureKit \
//     -framework AVFoundation -framework CoreMedia -framework CoreAudio

import Cocoa
import CryptoKit
import ScreenCaptureKit
import AVFoundation
import CoreMedia
import CoreAudio
import AudioToolbox

let HOME = FileManager.default.homeDirectoryForCurrentUser
let SERVICE_OWNER = ProcessInfo.processInfo.environment["YULU_SERVICE_OWNER"] ?? "unmanaged"
let PRODUCT_VERSION = Bundle.main.object(forInfoDictionaryKey: "OMMReleaseVersion") as? String ?? ""
let BUNDLE_VERSION = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? ""
let CAPTURE_IPC_VERSION = 1

func realDirectoryURL(_ path: String) -> URL? {
    var buffer = [CChar](repeating: 0, count: Int(PATH_MAX))
    let resolved = buffer.withUnsafeMutableBufferPointer { output in
        path.withCString { input in
            Darwin.realpath(input, output.baseAddress)
        }
    }
    guard resolved != nil else { return nil }
    return URL(fileURLWithPath: String(cString: buffer), isDirectory: true)
}

func canonicalDirectory(_ raw: String) -> URL? {
    let value = raw.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !value.isEmpty, !value.contains("\0") else { return nil }
    let expanded = value.hasPrefix("~/")
        ? HOME.appendingPathComponent(String(value.dropFirst(2)), isDirectory: true).path
        : value
    guard expanded.hasPrefix("/") else { return nil }
    let manager = FileManager.default
    var existing = URL(fileURLWithPath: expanded, isDirectory: true).standardizedFileURL
    var missing: [String] = []
    while true {
        var isDirectory: ObjCBool = false
        if manager.fileExists(atPath: existing.path, isDirectory: &isDirectory) {
            guard isDirectory.boolValue,
                  var resolved = realDirectoryURL(existing.path) else { return nil }
            for component in missing {
                resolved.appendPathComponent(component, isDirectory: true)
            }
            return resolved
        }
        if (try? manager.destinationOfSymbolicLink(atPath: existing.path)) != nil {
            return nil
        }
        let parent = existing.deletingLastPathComponent()
        guard parent.path != existing.path else { return nil }
        missing.insert(existing.lastPathComponent, at: 0)
        existing = parent
    }
}

func normalizedPathComponents(_ url: URL) -> [String] {
    url.standardizedFileURL.pathComponents
        .filter { $0 != "/" }
        .map { $0.precomposedStringWithCanonicalMapping.lowercased(with: Locale(identifier: "en_US_POSIX")) }
}

func isSameOrNested(_ candidate: URL, under root: URL) -> Bool {
    let path = normalizedPathComponents(candidate)
    let base = normalizedPathComponents(root)
    return path.count >= base.count && Array(path.prefix(base.count)) == base
}

func pathsOverlap(_ left: URL, _ right: URL) -> Bool {
    isSameOrNested(left, under: right) || isSameOrNested(right, under: left)
}

func environmentDirectory(_ name: String, fallback: URL) -> URL {
    guard let raw = ProcessInfo.processInfo.environment[name],
          raw.hasPrefix("/"),
          !raw.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
        return fallback
    }
    return URL(fileURLWithPath: raw, isDirectory: true).standardizedFileURL
}

// Require the Rust service's resolved XDG projection; never read Yulu's user directories.
func requiredDirectory(_ name: String) -> URL {
    guard let raw = ProcessInfo.processInfo.environment[name], raw.hasPrefix("/"),
          let url = realDirectoryURL(raw), url.path == raw else {
        fputs("OMM capture requires canonical directory overrides\n", stderr)
        exit(2)
    }
    var metadata = stat()
    guard Darwin.lstat(raw, &metadata) == 0, metadata.st_uid == geteuid(),
          (metadata.st_mode & 0o777) == 0o700 else {
        fputs("OMM capture requires private directories\n", stderr)
        exit(2)
    }
    return url
}
let DURABLE_DATA_DIR = requiredDirectory("YULU_APPLICATION_SUPPORT_DIR")
let IPC_DIR = requiredDirectory("YULU_IPC_DIR")
let LOGS_DIR = requiredDirectory("YULU_LOG_DIR")
let LEGACY_READ_ONLY_DATA_DIR = DURABLE_DATA_DIR
let CONFIG_DIR = DURABLE_DATA_DIR
let CONFIG_READ_PATHS = [DURABLE_DATA_DIR.appendingPathComponent("config.json")]
let SOCKET_PATH = IPC_DIR.appendingPathComponent("audio_daemon.sock")
let SOCKET_LOCK_PATH = IPC_DIR.appendingPathComponent(".audio_daemon.lock")
let STATE_PATH = DURABLE_DATA_DIR.appendingPathComponent(".state.json")
let PID_PATH = IPC_DIR.appendingPathComponent(".audio_daemon.pid")
let LOG_PATH = LOGS_DIR.appendingPathComponent("audio_daemon.log")
let DEFAULT_SILENCE_THRESHOLD: Float = 0.01
let DEFAULT_SILENCE_SEC: TimeInterval = 300
let SAMPLE_RATE: UInt32 = 48000
let DEFAULT_MIC_GAIN: Float = 2.4

var SYS_READY = false
/// When true, SCStream / ScreenCaptureKit is intentionally not started
/// (voicemail / dictation use case). The WAV's R channel stays at 0.
var SYS_DISABLED = false
var SYS_ERROR = ""
var MIC_READY = false
var MIC_ERROR = ""
var SYS_FORMAT_LOGGED = false

func defaultRecordingDir() -> URL {
    let candidate = requiredDirectory("YULU_MEDIA_LIBRARY_DIR")
    guard let safe = safeMediaDirectory(candidate.path) else {
        fatalError("no safe OMM recording directory")
    }
    return safe
}

func configuredRecordingDirectory(_ raw: String) -> URL? {
    safeMediaDirectory(raw)
}

func safeMediaDirectory(_ raw: String) -> URL? {
    guard let candidate = canonicalDirectory(raw) else { return nil }
    let operationalRoots = [DURABLE_DATA_DIR, IPC_DIR, LOGS_DIR, LEGACY_READ_ONLY_DATA_DIR]
    for root in operationalRoots {
        guard let canonicalRoot = canonicalDirectory(root.path) else { return nil }
        if pathsOverlap(candidate, canonicalRoot) { return nil }
    }
    return candidate
}

func loadRecordingDir() -> URL {
    _ = requiredDirectory("YULU_MEDIA_LIBRARY_DIR")
    if let raw = ProcessInfo.processInfo.environment["YULU_MEDIA_LIBRARY_DIR"],
       let configured = safeMediaDirectory(raw) {
        return configured
    }
    for configPath in CONFIG_READ_PATHS {
        guard let data = try? Data(contentsOf: configPath),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let audio = json["audio"] as? [String: Any],
              let raw = audio["output_dir"] as? String,
              let configured = configuredRecordingDirectory(raw) else {
            continue
        }
        return configured
    }
    return defaultRecordingDir()
}

let RECORDING_DIR = loadRecordingDir()

func safeRecordingSubdirectory(_ raw: String) -> URL? {
    guard let candidate = canonicalDirectory(raw),
          let recordingRoot = canonicalDirectory(RECORDING_DIR.path),
          isSameOrNested(candidate, under: recordingRoot) else {
        return nil
    }
    return candidate
}

var logFile: FileHandle?

// Operational diagnostics are fixed events, never imported free-form messages.
private enum CaptureOperationalEvent: String, CaseIterable {
    case tapCreateSuccess = "tap_create_success"
    case tapCreateError = "tap_create_error"
    case aggregateCreateError = "aggregate_create_error"
    case ioProcCreateError = "io_proc_create_error"
    case deviceStartError = "device_start_error"
    case systemNonzeroSeen = "system_nonzero_seen"
    case zeroRecovery = "zero_recovery"
    case zeroRecoverySuccess = "zero_recovery_success"
    case zeroRecoveryFailure = "zero_recovery_failure"
    case silenceThresholdReached = "silence_threshold_reached"
    case stopSilence = "stop_silence"
    case stopRequested = "stop_requested"
    case micGate = "mic_gate"
}

private func writeCaptureLog(_ suffix: String) {
    let df = DateFormatter(); df.dateFormat = "yyyy-MM-dd HH:mm:ss"
    let line = "[\(df.string(from: Date()))] capture \(suffix)"
    print(line); fflush(stdout)
    logFile?.write(Data((line + "\n").utf8))
}

func log(_ msg: String) {
    writeCaptureLog("event") // Never persist source paths, titles, UIDs or raw errors.
}

private func logEvent(_ event: CaptureOperationalEvent, code: Int32? = nil) {
    writeCaptureLog("event=\(event.rawValue)" + (code.map { " code=\($0)" } ?? ""))
}

// This bridge is capture recovery metadata, not the service's task database.
let BRIDGE_PATH = DURABLE_DATA_DIR.appendingPathComponent("omm-session.json")
var requestedFilename: String?
var bridgeError = ""
func readBridge() -> [String: Any] {
    guard let data = try? Data(contentsOf: BRIDGE_PATH),
          let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return [:] }
    return json
}
@discardableResult
func saveBridge(_ bridge: [String: Any]) -> Bool {
    do {
        let data = try JSONSerialization.data(withJSONObject: bridge)
        try data.write(to: BRIDGE_PATH, options: .atomic)
        guard chmod(BRIDGE_PATH.path, 0o600) == 0 else { throw NSError(domain: "capture", code: 1) }
        return true
    } catch {
        bridgeError = "capture_recovery_state_failed"
        return false
    }
}
@discardableResult
func appendBridgeFile(_ path: String, sequence: Int = 0, offsetFrames: Int = 0, segmentMs: Int = 1800000) -> Bool {
    var bridge = readBridge()
    var files = bridge["files"] as? [String] ?? []
    if !files.contains(path) { files.append(path) }
    bridge["files"] = files
    bridge["recording"] = true
    bridge["current_path"] = path; bridge["current_sequence"] = sequence; bridge["current_offset_frames"] = offsetFrames; bridge["segment_ms"] = segmentMs
    return saveBridge(bridge)
}

// EOF-framed, bounded, fixed internal service command; no shell/Python or audio upload.
func launchMeetingSilencePrompt() -> Bool {
    guard let path = ProcessInfo.processInfo.environment["OMM_SERVICE_SOCKET"],
          path.hasPrefix("/"), path.utf8.count < 104,
          let session = readBridge()["session_id"] as? String, UUID(uuidString: session) != nil else { return false }
    var socketMetadata = stat()
    guard Darwin.lstat(path, &socketMetadata) == 0,
          (socketMetadata.st_mode & S_IFMT) == S_IFSOCK,
          socketMetadata.st_uid == geteuid(), (socketMetadata.st_mode & 0o777) == 0o600 else { return false }
    let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
    guard fd >= 0 else { return false }
    defer { close(fd) }
    var timeout = timeval(tv_sec: 2, tv_usec: 0)
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
    var noPipe: Int32 = 1
    setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &noPipe, socklen_t(MemoryLayout<Int32>.size))
    var addr = sockaddr_un(); addr.sun_family = sa_family_t(AF_UNIX)
    path.withCString { _ = strncpy(&addr.sun_path.0, $0, 103) }
    let connected = withUnsafePointer(to: &addr) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
            Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) == 0
        }
    }
    guard connected, let data = try? JSONSerialization.data(withJSONObject: [
        "schema_version": 1, "command": "capture_auto_stop",
        "session_id": session, "instance_id": SERVICE_OWNER
    ]) else { return false }
    var sent = 0
    let written = data.withUnsafeBytes { bytes -> Bool in
        while sent < data.count {
            let n = Darwin.write(fd, bytes.baseAddress!.advanced(by: sent), data.count - sent)
            if n <= 0 { return false }; sent += n
        }
        return true
    }
    guard written, Darwin.shutdown(fd, SHUT_WR) == 0 else { return false }
    var reply = Data(); var buffer = [UInt8](repeating: 0, count: 1024)
    let deadline = Date(timeIntervalSinceNow: 2)
    while reply.count < 4096 {
        let remaining = deadline.timeIntervalSinceNow
        guard remaining > 0 else { return false }
        var readTimeout = timeval(tv_sec: Int(remaining), tv_usec: Int32((remaining - floor(remaining)) * 1_000_000))
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &readTimeout, socklen_t(MemoryLayout<timeval>.size))
        let n = Darwin.read(fd, &buffer, buffer.count)
        if n == 0 { break }; if n < 0 { return false }
        reply.append(buffer, count: n)
    }
    guard reply.count < 4096, let json = try? JSONSerialization.jsonObject(with: reply) as? [String: Any] else { return false }
    return json["ok"] as? Bool == true
}

// ─── WAV 写入器 ───────────────────────────────────────

final class AnchoredRecordingDirectory {
    let rootURL: URL
    let targetURL: URL
    private let rootFD: Int32
    private let targetFD: Int32
    private let rootMetadata: stat
    private let targetMetadata: stat

    init?(root: URL, target: URL) {
        let approvedRoot = root
        let approvedTarget = target
        guard isSameOrNested(approvedTarget, under: approvedRoot),
              let openedRoot = Self.openOrCreateAbsoluteDirectory(approvedRoot) else {
            return nil
        }
        var openedRootMetadata = stat()
        guard Darwin.fstat(openedRoot, &openedRootMetadata) == 0 else {
            Darwin.close(openedRoot)
            return nil
        }
        let rootComponents = approvedRoot.pathComponents
        let targetComponents = approvedTarget.pathComponents
        let relativeComponents = Array(targetComponents.dropFirst(rootComponents.count))
        guard let openedTarget = Self.openOrCreateRelativeDirectory(
            parentFD: openedRoot,
            components: relativeComponents
        ) else {
            Darwin.close(openedRoot)
            return nil
        }
        var openedTargetMetadata = stat()
        guard Darwin.fstat(openedTarget, &openedTargetMetadata) == 0,
              Self.pathStillNamesDirectory(approvedRoot, metadata: openedRootMetadata),
              Self.pathStillNamesDirectory(approvedTarget, metadata: openedTargetMetadata) else {
            Darwin.close(openedTarget)
            Darwin.close(openedRoot)
            return nil
        }
        rootURL = approvedRoot
        targetURL = approvedTarget
        rootFD = openedRoot
        targetFD = openedTarget
        rootMetadata = openedRootMetadata
        targetMetadata = openedTargetMetadata
    }

    deinit {
        Darwin.close(targetFD)
        Darwin.close(rootFD)
    }

    func createFile(named filename: String) -> (url: URL, handle: FileHandle)? {
        guard !filename.isEmpty,
              filename != ".",
              filename != "..",
              !filename.contains("/"),
              stillAnchored() else {
            return nil
        }
        let fd = filename.withCString {
            Darwin.openat(
                targetFD,
                $0,
                O_RDWR | O_CREAT | O_EXCL | O_NOFOLLOW,
                mode_t(0o600)
            )
        }
        guard fd >= 0 else { return nil }
        guard stillAnchored() else {
            Darwin.close(fd)
            filename.withCString { _ = Darwin.unlinkat(targetFD, $0, 0) }
            return nil
        }
        return (
            targetURL.appendingPathComponent(filename, isDirectory: false),
            FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        )
    }

    func removeFile(named filename: String) {
        filename.withCString { _ = Darwin.unlinkat(targetFD, $0, 0) }
    }

    private func stillAnchored() -> Bool {
        Self.pathStillNamesDirectory(rootURL, metadata: rootMetadata)
            && Self.pathStillNamesDirectory(targetURL, metadata: targetMetadata)
    }

    private static func openOrCreateAbsoluteDirectory(_ url: URL) -> Int32? {
        var directoryFD = Darwin.open("/", O_RDONLY | O_DIRECTORY | O_NOFOLLOW)
        guard directoryFD >= 0 else { return nil }
        for component in url.pathComponents where component != "/" {
            guard let nextFD = openOrCreateDirectory(parentFD: directoryFD, name: component) else {
                Darwin.close(directoryFD)
                return nil
            }
            Darwin.close(directoryFD)
            directoryFD = nextFD
        }
        return directoryFD
    }

    private static func openOrCreateRelativeDirectory(
        parentFD: Int32,
        components: [String]
    ) -> Int32? {
        var directoryFD = Darwin.dup(parentFD)
        guard directoryFD >= 0 else { return nil }
        for component in components {
            guard component != ".", component != "..", !component.contains("/"),
                  let nextFD = openOrCreateDirectory(parentFD: directoryFD, name: component) else {
                Darwin.close(directoryFD)
                return nil
            }
            Darwin.close(directoryFD)
            directoryFD = nextFD
        }
        return directoryFD
    }

    private static func openOrCreateDirectory(parentFD: Int32, name: String) -> Int32? {
        let flags = O_RDONLY | O_DIRECTORY | O_NOFOLLOW
        var opened = name.withCString { Darwin.openat(parentFD, $0, flags) }
        if opened < 0 && errno == ENOENT {
            let created = name.withCString { Darwin.mkdirat(parentFD, $0, mode_t(0o700)) }
            guard created == 0 || errno == EEXIST else { return nil }
            opened = name.withCString { Darwin.openat(parentFD, $0, flags) }
        }
        return opened >= 0 ? opened : nil
    }

    private static func pathStillNamesDirectory(_ url: URL, metadata: stat) -> Bool {
        var current = stat()
        let result = url.path.withCString { Darwin.lstat($0, &current) }
        return result == 0
            && (current.st_mode & S_IFMT) == S_IFDIR
            && current.st_dev == metadata.st_dev
            && current.st_ino == metadata.st_ino
    }
}

class WavWriter {
    let url: URL
    private let directory: AnchoredRecordingDirectory
    private var handle: FileHandle?
    private var audioSize: UInt32 = 0
    private var lastHeaderPatch = Date.distantPast
    private let lock = NSLock()

    init?(directory: AnchoredRecordingDirectory, filename: String) {
        guard let created = directory.createFile(named: filename) else { return nil }
        self.url = created.url
        self.directory = directory
        // 82 bytes = RIFF(12) + fmt chunk(24) + LIST-INFO-ICMT chunk(38) + data header(8)
        let h = created.handle
        do {
            try h.write(contentsOf: Data(repeating: 0, count: 82))
        } catch {
            try? h.close()
            directory.removeFile(named: filename)
            return nil
        }
        self.handle = h
        patchHeader(sync: true)
    }

    func append(_ data: Data) {
        lock.lock(); defer { lock.unlock() }
        guard let h = handle, !data.isEmpty else { return }
        do {
            try h.seekToEnd()
            try h.write(contentsOf: data)
            audioSize &+= UInt32(data.count)
            // Crash-resilience: keep the WAV header close to current size so a
            // force-kill still leaves a mostly playable file, not a 0-byte ghost.
            if Date().timeIntervalSince(lastHeaderPatch) >= 5 {
                patchHeaderLocked(sync: true)
            }
        } catch {
            log("WAV append failed: \(error)")
        }
    }

    func finalize() {
        lock.lock(); defer { lock.unlock() }
        patchHeaderLocked(sync: true)
        try? handle?.close()
        handle = nil
    }

    private func patchHeader(sync: Bool) {
        lock.lock(); defer { lock.unlock() }
        patchHeaderLocked(sync: sync)
    }

    private func patchHeaderLocked(sync: Bool) {
        let HDR_BYTES: UInt32 = 82
        let fileSize = audioSize + HDR_BYTES - 8  // RIFF size = total - 8

        var h = Data()
        // RIFF header
        h.append(contentsOf: [0x52,0x49,0x46,0x46] as [UInt8])   // "RIFF"
        var v32 = fileSize.littleEndian
        withUnsafeBytes(of: &v32) { h.append(Data($0)) }
        h.append(contentsOf: [0x57,0x41,0x56,0x45] as [UInt8])   // "WAVE"

        // fmt chunk
        h.append(contentsOf: [0x66,0x6D,0x74,0x20] as [UInt8])   // "fmt "
        v32 = UInt32(16).littleEndian
        withUnsafeBytes(of: &v32) { h.append(Data($0)) }
        var v16 = UInt16(1).littleEndian                          // PCM
        withUnsafeBytes(of: &v16) { h.append(Data($0)) }
        v16 = UInt16(2).littleEndian                              // channels=2
        withUnsafeBytes(of: &v16) { h.append(Data($0)) }
        v32 = UInt32(48000).littleEndian                          // sample rate
        withUnsafeBytes(of: &v32) { h.append(Data($0)) }
        v32 = UInt32(48000 * 2 * 2).littleEndian                  // byte rate
        withUnsafeBytes(of: &v32) { h.append(Data($0)) }
        v16 = UInt16(4).littleEndian                              // block align
        withUnsafeBytes(of: &v16) { h.append(Data($0)) }
        v16 = UInt16(16).littleEndian                             // bits/sample
        withUnsafeBytes(of: &v16) { h.append(Data($0)) }

        // LIST chunk with INFO/ICMT="Yulu DualTrack v1\0"
        h.append(contentsOf: [0x4C,0x49,0x53,0x54] as [UInt8])   // "LIST"
        v32 = UInt32(30).littleEndian                             // LIST body size
        withUnsafeBytes(of: &v32) { h.append(Data($0)) }
        h.append(contentsOf: [0x49,0x4E,0x46,0x4F] as [UInt8])   // "INFO"
        h.append(contentsOf: [0x49,0x43,0x4D,0x54] as [UInt8])   // "ICMT"
        v32 = UInt32(18).littleEndian                             // ICMT payload size
        withUnsafeBytes(of: &v32) { h.append(Data($0)) }
        // "Yulu DualTrack v1\0" = 18 bytes (even, no pad needed)
        h.append("Yulu DualTrack v1".data(using: .ascii)!)
        h.append(contentsOf: [0x00] as [UInt8])                   // null terminator

        // data chunk header
        h.append(contentsOf: [0x64,0x61,0x74,0x61] as [UInt8])   // "data"
        v32 = audioSize.littleEndian
        withUnsafeBytes(of: &v32) { h.append(Data($0)) }

        precondition(h.count == 82, "WAV header must be exactly 82 bytes (got \(h.count))")

        do {
            try handle?.seek(toOffset: 0)
            try handle?.write(contentsOf: h)
            try handle?.seekToEnd()
            if sync { handle?.synchronizeFile() }
            lastHeaderPatch = Date()
        } catch {
            log("WAV header patch failed: \(error)")
        }
    }
}

func normalizeToDBFS(_ samples: [Int16], targetDB: Float = -20) -> [Int16] {
    guard !samples.isEmpty else { return samples }
    var sum: Float = 0; let m = Float(Int(Int16.max) * Int(Int16.max))
    for s in samples { sum += Float(s) * Float(s) / m }
    let rms = sqrt(sum / Float(samples.count))
    guard rms > 0.0001 else { return samples }
    let gain = pow(10.0, targetDB / 20.0) / rms
    let clamped = min(max(gain, 0.1), 10.0)
    return samples.map { Int16(max(-1.0, min(1.0, Float($0) / Float(Int16.max) * clamped)) * Float(Int16.max)) }
}

// ─── 状态管理 ──────────────────────────────────────────

@discardableResult
func writeState(recording: Bool, title: String = "", path: String = "", sequence: Int = 0, offsetFrames: Int = 0, segmentMs: Int = 1800000) -> Bool {
    let df = ISO8601DateFormatter()
    var d: [String: Any] = [
        "version": 3,
        "sequence": sequence, "offset_frames": offsetFrames, "segment_ms": segmentMs,
        "recording": recording,
        "status": recording ? "recording" : "idle",
        "title": recording ? title : "",
        "file_path": recording ? path : "",
        "audio_path": recording ? path : "",
        "backend": "daemon",
        "started_at": recording ? df.string(from: Date()) : "",
        "updated_at": df.string(from: Date()),
    ]
    if recording, let prior = readStateDict(), (prior["recording"] as? Bool) == true {
        var segments = (prior["segments"] as? [String]) ?? []
        if let oldPath = (prior["audio_path"] as? String) ?? (prior["file_path"] as? String),
           !oldPath.isEmpty && !segments.contains(oldPath) {
            segments.append(oldPath)
        }
        if !path.isEmpty && !segments.contains(path) {
            segments.append(path)
        }
        d["segments"] = segments
        d["resume_count"] = prior["resume_count"] ?? 0
        d["meeting_id"] = prior["meeting_id"] ?? ""
    }
    do {
        let data = try JSONSerialization.data(withJSONObject: d, options: .prettyPrinted)
        try data.write(to: STATE_PATH, options: [.atomic])
        guard chmod(STATE_PATH.path, 0o600) == 0 else { throw NSError(domain: "capture", code: 1) }
        let file = try FileHandle(forWritingTo: STATE_PATH); defer { try? file.close() }; try file.synchronize()
        return true
    } catch { bridgeError = "capture_recovery_state_failed"; return false }
}

func readStateDict() -> [String: Any]? {
    guard let data = try? Data(contentsOf: STATE_PATH),
          let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        return nil
    }
    return obj
}

func interruptedRecordingTitle() -> String? {
    guard let state = readStateDict(),
          (state["recording"] as? Bool) == true,
          (state["backend"] as? String ?? "daemon") == "daemon" else {
        return nil
    }
    let title = (state["title"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    return title.isEmpty ? "meeting" : title
}

enum MeetingMicState: String {
    case unmuted
    case muted
    case unknown
}

struct MeetingMuteClassifier {
    private static func normalized(_ value: String) -> String {
        value.lowercased()
            .replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func actionMatches(_ label: String, _ action: String) -> Bool {
        label == action || label.hasPrefix(action + " ") || label.hasPrefix(action + "(")
    }

    static func classifyControlLabel(_ value: String) -> MeetingMicState? {
        let label = normalized(value)
        let ignored = [
            "mute all", "unmute all", "ask to unmute", "mute participant",
            "全体静音", "将所有人静音", "解除所有人静音", "请求解除静音",
        ]
        guard !label.isEmpty, !ignored.contains(where: label.contains) else { return nil }

        // Button labels describe the action, not the current state: an "Unmute"
        // button means the meeting microphone is currently muted.
        let mutedActions = [
            "unmute my audio", "unmute audio", "unmute", "turn on microphone",
            "start microphone", "解除静音", "取消静音", "开启麦克风", "打开麦克风",
            "麦克风已关闭",
        ]
        if mutedActions.contains(where: { actionMatches(label, $0) }) { return .muted }

        let unmutedActions = [
            "mute my audio", "mute audio", "mute", "turn off microphone",
            "stop microphone", "静音", "关闭麦克风", "麦克风已开启",
        ]
        if unmutedActions.contains(where: { actionMatches(label, $0) }) { return .unmuted }
        return nil
    }

    static func isSupportedApp(appName: String, bundleID: String?) -> Bool {
        let app = normalized("\(appName) \(bundleID ?? "")")
        return [
            "zoom.us", "zoom workplace", "us.zoom.xos",
            "腾讯会议", "tencent meeting", "tencentmeeting", "voov meeting", "wemeet", "com.tencent.meeting",
            "google chrome", "com.google.chrome", " arc ", "company.thebrowser.browser",
            "safari", "microsoft edge", "com.microsoft.edgemac",
        ].contains(where: app.contains)
    }

    static func isSupportedMeetingWindow(appName: String, bundleID: String?, title: String) -> Bool {
        let app = normalized("\(appName) \(bundleID ?? "")")
        if ["zoom.us", "zoom workplace", "us.zoom.xos"].contains(where: app.contains) {
            return true
        }
        if ["腾讯会议", "tencent meeting", "tencentmeeting", "voov meeting", "wemeet", "com.tencent.meeting"].contains(where: app.contains) {
            return true
        }
        let browser = [
            "google chrome", "com.google.chrome", " arc ", "company.thebrowser.browser",
            "safari", "microsoft edge", "com.microsoft.edgemac",
        ].contains(where: app.contains)
        let window = normalized(title)
        return browser && (window.contains("google meet") || window.contains("meet.google.com") || window.hasPrefix("meet -"))
    }
}

func micSamplesForRecording(_ samples: [Float], gain: Float, meetingState: MeetingMicState) -> [Int16] {
    if meetingState == .muted {
        return [Int16](repeating: 0, count: samples.count)
    }
    return samples.map { Int16(max(-1.0, min(1.0, $0 * gain)) * Float(Int16.max)) }
}

// ─── 音频数据管理器 + 源分离立体声 (L=mic, R=sys) ───

class AudioRecorder {
    private let recorderQueue = DispatchQueue(label: "org.openmeetingmemo.audioRecorder")
    private let recorderQueueKey = DispatchSpecificKey<Bool>()

    private var writer: RollingRecordingWriter?
    private var maxSegmentFrames = 1800 * 48000
    private var isRecordingState = false
    private var startTime: Date?
    private var lastMicAudioTime: Date?
    private var lastSysAudioTime: Date?
    private var silenceTimer: DispatchSourceTimer?
    private var silenceSecondsState = DEFAULT_SILENCE_SEC
    private var silenceThresholdState = DEFAULT_SILENCE_THRESHOLD
    private var outputDirState: URL = RECORDING_DIR
    var onStopRequest: (() -> Void)?
    private var sysGapMicFallbackLogged = false
    private var systemNonzeroSeen = false
    private var micGainState: Float = DEFAULT_MIC_GAIN
    private var micLevelState: Float = 0
    private var meetingMicStateState: MeetingMicState = .unknown
    private var autoStopRequested = false
    private let micRateAdapter = CaptureInputRateAdapter()
    private let sysRateAdapter = CaptureInputRateAdapter()
    private var sourceClocks: [CaptureInputSource: CaptureSourceClock] = [:]
    private var sourceFormatRejects: UInt64 = 0
    private var captureStartedUs: UInt64 = 0
    private var captureStartedUtcMs: Int64 = 0

    var captureClock: [String: Any] {
        syncState {
            var result: [String: Any] = ["schema_version": 1, "target_rate_hz": 48000,
                "writer_started_utc_ms": captureStartedUtcMs, "elapsed_us": captureStartedUs == 0 ? 0 : DispatchTime.now().uptimeNanoseconds / 1000 - captureStartedUs,
                "format_rejects": sourceFormatRejects]
            for (source, clock) in sourceClocks {
                var fields = clock.document(startedUs: captureStartedUs)
                fields["accepted_channels"] = source == .mic ? 1 : 2
                result[source.rawValue] = fields
            }
            return result
        }
    }

    func rejectSourceFormat() {
        recorderQueue.async { [weak self] in
            guard let self = self, self.isRecordingState else { return }
            self.sourceFormatRejects += 1
        }
    }

    private func noteInput(_ source: CaptureInputSource, rate: Double, inputFrames: Int, targetFrames: Int) {
        let now = DispatchTime.now().uptimeNanoseconds / 1000
        var clock = sourceClocks[source] ?? CaptureSourceClock(rate: rate, firstInputUs: now, lastInputUs: now)
        if clock.rate != rate { clock.rateChanges += 1; clock.rate = rate }
        clock.inputFrames += UInt64(inputFrames); clock.targetFrames += UInt64(targetFrames)
        clock.lastInputUs = now; sourceClocks[source] = clock
    }

    // Streaming buffers
    private var sysBuf: [Int16] = []
    private var micBuf: [Int16] = []

    init() {
        recorderQueue.setSpecific(key: recorderQueueKey, value: true)
    }

    private func onRecorderQueue() -> Bool {
        DispatchQueue.getSpecific(key: recorderQueueKey) == true
    }

    private func syncState<T>(_ body: () -> T) -> T {
        if onRecorderQueue() {
            return body()
        }
        return recorderQueue.sync(execute: body)
    }

    var isRecording: Bool {
        syncState { isRecordingState }
    }

    var currentFilePath: String {
        syncState { writer?.url.path ?? "" }
    }

    var micGain: Float {
        syncState { micGainState }
    }

    var micLevel: Float {
        syncState { micLevelState }
    }

    var meetingMicState: MeetingMicState {
        syncState { meetingMicStateState }
    }

    func updateMeetingMicState(_ state: MeetingMicState) {
        recorderQueue.async { [weak self] in
            guard let self = self, self.meetingMicStateState != state else { return }
            self.meetingMicStateState = state
            guard self.isRecordingState else { return }
            logEvent(.micGate, code: state == .muted ? 1 : (state == .unmuted ? 0 : -1))
            switch state {
            case .muted:
                log("🎤 Meeting mic muted — microphone track suppressed")
            case .unmuted:
                log("🎤 Meeting mic unmuted — microphone track recording")
            case .unknown:
                log("🎤 Meeting mic state unknown — microphone track recording")
            }
        }
    }

    func configure(silenceSeconds: TimeInterval, silenceThreshold: Float, outputDir: URL, segmentMs: Int = 1800000) {
        syncState {
            silenceSecondsState = silenceSeconds
            silenceThresholdState = silenceThreshold
            outputDirState = outputDir
            maxSegmentFrames = min(1800000, max(60000, segmentMs)) * 48
        }
    }

    func start(title: String, sequence: Int = 0, offsetFrames: Int = 0) -> String? {
        return syncState {
            if !bridgeError.isEmpty { return nil }
            if isRecordingState {
                return writer?.url.path
            }

            let df = DateFormatter(); df.dateFormat = "yyyyMMdd_HHmmss"
            let fn = requestedFilename ?? "\(title.components(separatedBy: .alphanumerics.inverted).joined())_\(df.string(from: Date())).wav"
            guard let directory = AnchoredRecordingDirectory(
                root: RECORDING_DIR,
                target: outputDirState
            ), let w = RollingRecordingWriter(directory: directory, filename: fn, maxFrames: maxSegmentFrames,
                                                sequence: sequence, offsetFrames: offsetFrames, title: title) else {
                log("Recording directory changed or is unsafe: \(outputDirState.path)")
                return nil
            }
            let url = w.url
            writer = w
            micRateAdapter.reset(); sysRateAdapter.reset(); sourceClocks = [:]; sourceFormatRejects = 0
            captureStartedUs = DispatchTime.now().uptimeNanoseconds / 1000
            captureStartedUtcMs = Int64(Date().timeIntervalSince1970 * 1000)
            w.captureClock = { [weak self] in self?.captureClock ?? [:] }
            isRecordingState = true
            startTime = Date()
            lastMicAudioTime = Date()
            lastSysAudioTime = Date()
            sysBuf = []
            micBuf = []
            micLevelState = 0
            sysGapMicFallbackLogged = false
            systemNonzeroSeen = false
            meetingMicStateState = .unknown
            autoStopRequested = false
            guard writeState(recording: true, title: title, path: url.path, sequence: sequence,
                             offsetFrames: offsetFrames, segmentMs: maxSegmentFrames / 48) else {
                isRecordingState = false; return nil
            }
            log("🎙 \(fn)")
            startSilenceMonitorOnQueue()
            return url.path
        }
    }

    @discardableResult
    func stop() -> (path: String?, duration: Int) {
        return syncState {
            if isRecordingState { logEvent(.stopRequested) }
            isRecordingState = false
            stopSilenceMonitorOnQueue()
            let dur = startTime.map { Int(Date().timeIntervalSince($0)) } ?? 0
            if let tail = micRateAdapter.finish() {
                micBuf.append(contentsOf: micSamplesForRecording(tail, gain: micGainState, meetingState: meetingMicStateState))
                sourceClocks[.mic]?.targetFrames += UInt64(tail.count)
            } else { sourceFormatRejects += 1 }
            if let tail = sysRateAdapter.finish() {
                sysBuf.append(contentsOf: tail.map { Int16(max(-1, min(1, $0)) * Float(Int16.max)) })
                sourceClocks[.system]?.targetFrames += UInt64(tail.count / 2)
            } else { sourceFormatRejects += 1 }
            flushBuffersOnQueue()
            let p = writer?.url.path
            writer?.finalize()
            guard writer?.failed != true, writeState(recording: false) else {
                bridgeError = "capture_recovery_state_failed"; return (p, dur)
            }
            bridgeError = ""
            writer = nil
            startTime = nil
            lastMicAudioTime = nil
            lastSysAudioTime = nil
            sysBuf = []
            micBuf = []
            micLevelState = 0
            meetingMicStateState = .unknown
            writeState(recording: false)
            log("⏹ \(dur)s")
            return (p, dur)
        }
    }

    func onSysAudio(_ samples: [Int16], sourceRate: Double = 48000) {
        recorderQueue.async { [weak self] in
            guard let self = self, self.isRecordingState else { return }
            guard samples.count % 2 == 0, sourceRate.isFinite, (8000...192000).contains(sourceRate) else {
                self.sourceFormatRejects += 1; return
            }
            let normalized: [Int16]
            if sourceRate == 48000 {
                guard let tail = self.sysRateAdapter.finish() else { self.sourceFormatRejects += 1; return }
                normalized = tail.isEmpty ? samples : tail.map { Int16(max(-1, min(1, $0)) * Float(Int16.max)) } + samples
            } else {
                guard let converted = self.sysRateAdapter.convert(samples.map { Float($0) / Float(Int16.max) }, rate: sourceRate, channels: 2) else {
                    self.sourceFormatRejects += 1; return
                }
                normalized = converted.map { Int16(max(-1, min(1, $0)) * Float(Int16.max)) }
            }
            self.noteInput(.system, rate: sourceRate, inputFrames: samples.count / 2, targetFrames: normalized.count / 2)
            if !self.systemNonzeroSeen && samples.contains(where: { $0 != 0 }) {
                self.systemNonzeroSeen = true
                logEvent(.systemNonzeroSeen)
            }
            self.sysBuf.append(contentsOf: normalized)
            self.sysGapMicFallbackLogged = false
            if self.calcRMS(samples) > self.silenceThresholdState {
                self.lastSysAudioTime = Date()
            }
            self.mixAndWriteOnQueue()
        }
    }

    func onMicAudio(_ samples: [Float], sourceRate: Double = 48000) {
        recorderQueue.async { [weak self] in
            guard let self = self, self.isRecordingState else { return }
            guard let normalized = self.micRateAdapter.convert(samples, rate: sourceRate, channels: 1) else {
                self.sourceFormatRejects += 1; return
            }
            self.noteInput(.mic, rate: sourceRate, inputFrames: samples.count, targetFrames: normalized.count)
            let gain = self.micGainState
            let ints = micSamplesForRecording(normalized, gain: gain, meetingState: self.meetingMicStateState)
            self.micBuf.append(contentsOf: ints)
            let rms = self.calcRMS(ints)
            self.micLevelState = self.meetingMicStateState == .muted ? 0 : max(rms, self.micLevelState * 0.72)
            if self.meetingMicStateState != .muted && rms > self.silenceThresholdState {
                self.lastMicAudioTime = Date()
            }
            self.mixAndWriteOnQueue()
        }
    }

    func noteCaptureRouteChange(now: Date = Date()) {
        syncState {
            guard isRecordingState else { return }
            var bridge = readBridge()
            var gaps = bridge["gaps"] as? [[String: Any]] ?? []
            let wallTimeMs = Int(now.timeIntervalSince1970 * 1000)
            let startedWallMs = bridge["started_wall_ms"] as? Int ?? Int((startTime ?? now).timeIntervalSince1970 * 1000)
            gaps.append(["frame_offset": writer?.framePosition ?? 0, "wall_time_ms": wallTimeMs,
                         "wall_offset_ms": max(0, wallTimeMs - startedWallMs), "kind": "microphone_route_restart"])
            bridge["gaps"] = gaps
            if !saveBridge(bridge) { bridgeError = "capture_recovery_state_failed" }
            lastMicAudioTime = now
            lastSysAudioTime = now
        }
    }

    func silenceExpired(now: Date = Date()) -> Bool {
        return syncState {
            silenceExpiredOnQueue(now: now)
        }
    }

    func _selfTestSetSilenceState(recording: Bool, micLast: Date?, sysLast: Date?, silenceSeconds: TimeInterval = DEFAULT_SILENCE_SEC) {
        syncState {
            isRecordingState = recording
            lastMicAudioTime = micLast
            lastSysAudioTime = sysLast
            silenceSecondsState = silenceSeconds
        }
    }

    private func calcRMS(_ s: [Int16]) -> Float {
        guard !s.isEmpty else { return 0 }
        var sum: Float = 0; let m = Float(Int(Int16.max) * Int(Int16.max))
        for v in s { sum += Float(v) * Float(v) / m }
        return sqrt(sum / Float(s.count))
    }

    /// Produce an interleaved stereo PCM stream where L=mic mono and
    /// R=(sysL + sysR)/2 downmixed to mono. Source-separated; no ducking.
    /// Inputs:
    ///   - sysStereo: interleaved Int16 stereo (length must be even)
    ///   - micMono:   Int16 mono samples
    /// Output length is `sysStereo.count` (one stereo Int16 per frame).
    private func channelInterleave(sysStereo: [Int16], micMono: [Int16]) -> [Int16] {
        let frames = sysStereo.count / 2
        var out = [Int16](repeating: 0, count: frames * 2)
        let micFrames = micMono.count

        for i in 0..<frames {
            // L = mic mono (zero-padded if mic short)
            out[2 * i] = i < micFrames ? micMono[i] : 0
            // R = (sysL + sysR) / 2 with overflow-safe Int32 widening
            let sysL = Int32(sysStereo[2 * i])
            let sysR = Int32(sysStereo[2 * i + 1])
            out[2 * i + 1] = Int16((sysL + sysR) / 2)
        }
        return out
    }

    /// 流式 source-separated: 从 sysBuf/micBuf 取等量数据,
    /// 写出 L=mic / R=sys downmix(L+R/2) 的立体声 PCM.
    private func mixAndWriteOnQueue() {
        guard let w = writer else { return }

        let sysFrames = sysBuf.count / 2
        let micFrames = micBuf.count
        let micOnly = SYS_DISABLED
        // If the process tap is stuck delivering all-zero buffers, sysBuf never
        // receives frames. Keep preserving the mic track instead of letting it
        // pile up in memory until a force-kill leaves only an 82-byte WAV header.
        let forceMicOnly = !micOnly && sysFrames == 0 && micFrames >= Int(SAMPLE_RATE)
        // In normal meeting recordings the mic and system callbacks arrive on
        // independent queues. Driving output by the longer buffer turns
        // each early callback into a zero-padded chunk, stretching the timeline
        // into audible sys-only/mic-only stutter. Only true mic-only recordings
        // and sys-gap fallback recordings may be driven by mic frames; ordinary
        // dual-source recordings write common frames and leave the short side
        // buffered for the next callback.
        let outFrames = (micOnly || forceMicOnly) ? micFrames : min(sysFrames, micFrames)
        guard outFrames >= 512 else { return }  // wait for ~10 ms

        let micChunk = Array(micBuf.prefix(outFrames))
        micBuf.removeFirst(outFrames)

        let sysChunk: [Int16]
        if micOnly || forceMicOnly {
            sysChunk = [Int16](repeating: 0, count: outFrames * 2)
        } else {
            sysChunk = Array(sysBuf.prefix(outFrames * 2))
            sysBuf.removeFirst(outFrames * 2)
        }
        let shouldLogFallback = forceMicOnly && !sysGapMicFallbackLogged
        if forceMicOnly { sysGapMicFallbackLogged = true }

        if shouldLogFallback {
            log("⚠️ Sys audio unavailable; preserving mic-only audio until system audio resumes")
        }
        let out = channelInterleave(sysStereo: sysChunk, micMono: micChunk)
        w.append(Data(bytes: out, count: out.count * 2))
    }

    private func flushBuffersOnQueue() {
        guard let w = writer else { return }
        let sysFrames = sysBuf.count / 2
        let micFrames = micBuf.count
        let outFrames = max(sysFrames, micFrames)
        if outFrames == 0 { return }

        let micChunk: [Int16] = micBuf + [Int16](repeating: 0, count: max(0, outFrames - micFrames))
        let sysChunk: [Int16] = sysBuf + [Int16](repeating: 0, count: max(0, outFrames * 2 - sysBuf.count))
        micBuf.removeAll()
        sysBuf.removeAll()

        let out = channelInterleave(sysStereo: Array(sysChunk.prefix(outFrames * 2)),
                                     micMono:   Array(micChunk.prefix(outFrames)))
        w.append(Data(bytes: out, count: out.count * 2))
    }

    private func silenceExpiredOnQueue(now: Date = Date()) -> Bool {
        guard isRecordingState else { return false }
        let seconds = silenceSecondsState
        let micQuiet = (lastMicAudioTime.map { now.timeIntervalSince($0) } ?? .infinity) >= seconds
        let sysQuiet = (lastSysAudioTime.map { now.timeIntervalSince($0) } ?? .infinity) >= seconds
        return micQuiet && sysQuiet
    }

    private func startSilenceMonitorOnQueue() {
        guard silenceTimer == nil else { return }
        let timer = DispatchSource.makeTimerSource(queue: recorderQueue)
        timer.schedule(deadline: .now() + 1, repeating: 1)
        timer.setEventHandler { [weak self] in
            guard let self = self, self.silenceExpiredOnQueue() else { return }
            guard !self.autoStopRequested else { return }
            self.autoStopRequested = true
            logEvent(.silenceThresholdReached)
            let action = SYS_DISABLED ? "auto stop" : "confirm before stop"
            log("🔇 silence \(Int(self.silenceSecondsState))s (both channels) — \(action)")
            DispatchQueue.main.async { [weak self] in
                self?.onStopRequest?()
            }
        }
        silenceTimer = timer
        timer.resume()
    }

    private func stopSilenceMonitorOnQueue() {
        silenceTimer?.setEventHandler {}
        silenceTimer?.cancel()
        silenceTimer = nil
        autoStopRequested = false
    }
}

final class MeetingMuteMonitor {
    private let recorder: AudioRecorder
    private let queue = DispatchQueue(label: "org.openmeetingmemo.meeting-mute-monitor", qos: .utility)
    private var timer: DispatchSourceTimer?

    init(recorder: AudioRecorder) {
        self.recorder = recorder
    }

    func start() {
        queue.async { [weak self] in
            guard let self = self, self.timer == nil else { return }
            let timer = DispatchSource.makeTimerSource(queue: self.queue)
            timer.schedule(deadline: .now(), repeating: 1)
            timer.setEventHandler { [weak self] in
                guard let self = self else { return }
                self.recorder.updateMeetingMicState(self.scan())
            }
            self.timer = timer
            timer.resume()
        }
    }

    func stop() {
        queue.async { [weak self] in
            guard let self = self else { return }
            self.timer?.setEventHandler {}
            self.timer?.cancel()
            self.timer = nil
            self.recorder.updateMeetingMicState(.unknown)
        }
    }

    private func scan() -> MeetingMicState {
        guard AXIsProcessTrusted() else { return .unknown }
        for app in NSWorkspace.shared.runningApplications where app.activationPolicy == .regular {
            guard MeetingMuteClassifier.isSupportedApp(
                appName: app.localizedName ?? "",
                bundleID: app.bundleIdentifier
            ) else { continue }
            let appElement = AXUIElementCreateApplication(app.processIdentifier)
            AXUIElementSetMessagingTimeout(appElement, 0.2)
            var windowsRef: CFTypeRef?
            guard AXUIElementCopyAttributeValue(appElement, kAXWindowsAttribute as CFString, &windowsRef) == .success,
                  let windows = windowsRef as? [AXUIElement] else { continue }
            for window in windows {
                let title = stringAttribute(window, kAXTitleAttribute as CFString) ?? ""
                guard MeetingMuteClassifier.isSupportedMeetingWindow(
                    appName: app.localizedName ?? "",
                    bundleID: app.bundleIdentifier,
                    title: title
                ) else { continue }
                if let state = muteState(in: window) { return state }
            }
        }
        return .unknown
    }

    private func muteState(in root: AXUIElement) -> MeetingMicState? {
        var stack: [(AXUIElement, Int)] = [(root, 0)]
        var visited = 0
        while let (element, depth) = stack.popLast(), visited < 600 {
            visited += 1
            let role = stringAttribute(element, kAXRoleAttribute as CFString) ?? ""
            if ["AXButton", "AXCheckBox", "AXMenuItem", "AXRadioButton", "AXPopUpButton"].contains(role) {
                for attribute in [kAXTitleAttribute, kAXDescriptionAttribute, kAXHelpAttribute] {
                    if let label = stringAttribute(element, attribute as CFString),
                       let state = MeetingMuteClassifier.classifyControlLabel(label) {
                        return state
                    }
                }
            }
            guard depth < 12 else { continue }
            var childrenRef: CFTypeRef?
            if AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &childrenRef) == .success,
               let children = childrenRef as? [AXUIElement] {
                for child in children.reversed() { stack.append((child, depth + 1)) }
            }
        }
        return nil
    }

    private func stringAttribute(_ element: AXUIElement, _ attribute: CFString) -> String? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute, &value) == .success else { return nil }
        return value as? String
    }
}

private enum CaptureInputFormatFault: String, Error {
    case unavailable = "capture_source_format_unavailable"
    case invalid = "capture_source_format_invalid"
    case changed = "capture_source_format_changed"
    case buffer = "capture_source_buffer_format_invalid"
    static func startError(_ sourceError: String) -> String {
        CaptureInputFormatFault(rawValue: sourceError)?.rawValue ?? "sys_capture_not_ready"
    }
    static func stopError(_ sourceError: String, clock: [String: Any]) -> String? {
        let rejected = clock["format_rejects"] as? UInt64 ?? 0
        guard let fault = CaptureInputFormatFault(rawValue: sourceError) ?? (rejected > 0 ? .buffer : nil) else { return nil }
        let accepted = ["mic", "system"].contains { (clock[$0] as? [String: Any])?["target_frames"] as? UInt64 ?? 0 > 0 }
        return accepted ? nil : fault.rawValue
    }
}

private struct CapturePCMFormat {
    let rate: Double
    let channels: Int
    let planar: Bool

    init?(_ a: AudioStreamBasicDescription) {
        let planar = a.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0
        let channels = Int(a.mChannelsPerFrame)
        guard a.mFormatID == kAudioFormatLinearPCM, a.mSampleRate.isFinite,
              (8000...192000).contains(a.mSampleRate), (1...2).contains(channels),
              a.mFormatFlags & kAudioFormatFlagIsFloat != 0,
              a.mFormatFlags & (kAudioFormatFlagIsBigEndian | kAudioFormatFlagIsSignedInteger) == 0,
              a.mBitsPerChannel == 32, a.mFramesPerPacket == 1,
              a.mBytesPerFrame == UInt32((planar ? 1 : channels) * 4),
              a.mBytesPerPacket == a.mBytesPerFrame else { return nil }
        self.rate = a.mSampleRate; self.channels = channels; self.planar = planar
    }

    func accepts(_ buffers: UnsafeMutableAudioBufferListPointer) -> Bool {
        guard buffers.count == (planar ? channels : 1) else { return false }
        let width = (planar ? 1 : channels) * MemoryLayout<Float>.size
        let bytes = Int(buffers[0].mDataByteSize)
        guard bytes % width == 0, bytes / width <= 262144 else { return false }
        return buffers.allSatisfy {
            $0.mNumberChannels == UInt32(planar ? 1 : channels)
                && Int($0.mDataByteSize) == bytes && (bytes == 0 || $0.mData != nil)
        }
    }
}

// The IOProc block retains its own immutable format, never a mutable global tap ID.
// Only the active bit is shared; Darwin's atomic primitive adds no callback lock.
private final class TapIOFormatContext {
    let format: CapturePCMFormat
    let generation: UInt64
    private var active: Int32 = 1
    init(format: CapturePCMFormat, generation: UInt64) {
        self.format = format; self.generation = generation
    }
    var isActive: Bool { OSAtomicAdd32Barrier(0, &active) == 1 }
    @discardableResult func invalidate() -> Bool { OSAtomicCompareAndSwap32Barrier(1, 0, &active) }
}

// Owned by the non-realtime lifecycle lock. Provider injection exercises the same
// validation/generation path as buildTap, without creating a device or requesting TCC.
private final class TapInputFormatLifecycle {
    private(set) var current: TapIOFormatContext?
    private(set) var fault: CaptureInputFormatFault?
    private(set) var ready = false
    private var generation: UInt64 = 0
    func invalidate() { current?.invalidate(); current = nil; ready = false }
    func didStart(_ context: TapIOFormatContext) -> Bool {
        guard current === context, context.isActive else { return false }
        ready = true; fault = nil; return true
    }
    func prepare(_ provider: () -> Result<AudioStreamBasicDescription, CaptureInputFormatFault>) -> TapIOFormatContext? {
        invalidate()
        let description: AudioStreamBasicDescription
        switch provider() {
        case .success(let value): description = value
        case .failure(let reason): fault = reason; return nil
        }
        guard let format = CapturePCMFormat(description) else { fault = .invalid; return nil }
        generation &+= 1
        let context = TapIOFormatContext(format: format, generation: generation)
        current = context; fault = nil
        return context
    }
}

// Adapt the device clock before the imported 48 kHz PCM pairing/WAV kernel.
final class CaptureInputRateAdapter {
    private var converter: AVAudioConverter?
    private var inputRate: Double = 0
    private var channelCount = 0
    private var inputFrames: UInt64 = 0
    private var emittedFrames: UInt64 = 0

    func reset() { converter = nil; inputRate = 0; channelCount = 0; inputFrames = 0; emittedFrames = 0 }

    func convert(_ samples: [Float], rate: Double, channels: Int) -> [Float]? {
        guard rate.isFinite, (8000...192000).contains(rate), (1...2).contains(channels),
              samples.count % channels == 0, samples.count / channels <= 262144 else { return nil }
        if samples.isEmpty { return [] }
        var previous: [Float] = []
        if rate != inputRate || channels != channelCount {
            guard let tail = finish() else { return nil }; previous = tail
        }
        if rate == 48000 { return previous.isEmpty ? samples : previous + samples }
        if converter == nil {
            guard let input = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: rate,
                                            channels: AVAudioChannelCount(channels), interleaved: true),
                  let output = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 48000,
                                             channels: AVAudioChannelCount(channels), interleaved: true),
                  let next = AVAudioConverter(from: input, to: output) else { return nil }
            next.primeMethod = .none
            converter = next; inputRate = rate; channelCount = channels
        }
        guard let converter = converter else { return nil }
        let frames = samples.count / channels
        inputFrames += UInt64(frames)
        guard let input = AVAudioPCMBuffer(pcmFormat: converter.inputFormat,
                                           frameCapacity: AVAudioFrameCount(frames)),
              let pointer = input.mutableAudioBufferList.pointee.mBuffers.mData else { return nil }
        input.frameLength = AVAudioFrameCount(frames)
        samples.withUnsafeBytes { pointer.copyMemory(from: $0.baseAddress!, byteCount: $0.count) }
        let capacity = AVAudioFrameCount(ceil(Double(frames) * 48000 / rate)) + 256
        guard let output = AVAudioPCMBuffer(pcmFormat: converter.outputFormat,
                                            frameCapacity: capacity) else { return nil }
        var supplied = false
        var converted: [Float] = previous
        // noDataNow preserves the converter's fractional phase across callbacks; endOfStream would reset it.
        for _ in 0..<4 {
            output.frameLength = 0
            var error: NSError?
            let status = converter.convert(to: output, error: &error) { _, state in
                if supplied { state.pointee = .noDataNow; return nil }
                supplied = true; state.pointee = .haveData; return input
            }
            guard error == nil, status != .error,
                  let data = output.audioBufferList.pointee.mBuffers.mData else { return nil }
            emittedFrames += UInt64(output.frameLength)
            converted.append(contentsOf: UnsafeBufferPointer(start: data.assumingMemoryBound(to: Float.self),
                                                            count: Int(output.frameLength) * channels))
            if status != .haveData { return converted }
        }
        return nil
    }

    func finish() -> [Float]? {
        guard let converter = converter else { return [] }
        defer { reset() }
        guard let output = AVAudioPCMBuffer(pcmFormat: converter.outputFormat, frameCapacity: 4096) else { return nil }
        var result: [Float] = []
        for _ in 0..<4 {
            output.frameLength = 0
            var error: NSError?
            let status = converter.convert(to: output, error: &error) { _, state in
                state.pointee = .endOfStream; return nil
            }
            guard error == nil, status != .error,
                  let data = output.audioBufferList.pointee.mBuffers.mData else { return nil }
            result.append(contentsOf: UnsafeBufferPointer(start: data.assumingMemoryBound(to: Float.self), count: Int(output.frameLength) * channelCount))
            if status == .endOfStream {
                // End-of-stream includes filter padding. Keep the real input clock, not that padding.
                let expected = UInt64((Double(inputFrames) * 48000 / inputRate).rounded())
                let remaining = expected > emittedFrames ? expected - emittedFrames : 0
                return Array(result.prefix(Int(remaining) * channelCount))
            }
        }
        return nil
    }
}

enum CaptureInputSource: String { case mic, system }
struct CaptureSourceClock {
    var rate: Double
    var inputFrames: UInt64 = 0
    var targetFrames: UInt64 = 0
    var rateChanges = 0
    var firstInputUs: UInt64
    var lastInputUs: UInt64
    func document(startedUs: UInt64) -> [String: Any] {
        ["rate_hz": rate, "input_frames": inputFrames, "target_frames": targetFrames,
         "rate_changes": rateChanges, "input_elapsed_us": lastInputUs - firstInputUs,
         "first_accepted_offset_us": firstInputUs - startedUs, "last_accepted_offset_us": lastInputUs - startedUs]
    }
}

class MicCapture {
    let recorder: AudioRecorder
    private let lifecycleQueue = DispatchQueue(label: "org.openmeetingmemo.mic-capture.lifecycle")
    private var engine: AVAudioEngine?
    private var configurationObserver: NSObjectProtocol?
    private var generation: UInt64 = 0
    var selectedDeviceUID: String?

    init(recorder: AudioRecorder) { self.recorder = recorder }

    private static func stringProperty(_ device: AudioObjectID, selector: AudioObjectPropertySelector) -> String? {
        var address = AudioObjectPropertyAddress(
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var size = UInt32(MemoryLayout<CFString?>.size)
        let pointer = UnsafeMutablePointer<CFString?>.allocate(capacity: 1)
        pointer.initialize(to: nil)
        defer {
            pointer.deinitialize(count: 1)
            pointer.deallocate()
        }
        guard AudioObjectGetPropertyData(device, &address, 0, nil, &size, pointer) == noErr,
              let value = pointer.pointee else { return nil }
        return value as String
    }

    private static func hasStreams(_ device: AudioObjectID, scope: AudioObjectPropertyScope) -> Bool {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioDevicePropertyStreams,
            mScope: scope,
            mElement: kAudioObjectPropertyElementMain
        )
        var size: UInt32 = 0
        return AudioObjectGetPropertyDataSize(device, &address, 0, nil, &size) == noErr && size > 0
    }

    static func availableDevices() -> [String: Any] {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyDevices,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size) == noErr else {
            return ["error": "coreaudio_device_size_failed"]
        }
        var devices = [AudioDeviceID](repeating: 0, count: Int(size) / MemoryLayout<AudioDeviceID>.size)
        guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &devices) == noErr else {
            return ["error": "coreaudio_device_list_failed"]
        }
        var input: [[String: String]] = []
        var output: [[String: String]] = []
        for device in devices {
            guard let uid = stringProperty(device, selector: kAudioDevicePropertyDeviceUID) else { continue }
            let item = ["uid": uid, "name": stringProperty(device, selector: kAudioObjectPropertyName) ?? uid]
            if hasStreams(device, scope: kAudioDevicePropertyScopeInput) { input.append(item) }
            if hasStreams(device, scope: kAudioDevicePropertyScopeOutput) { output.append(item) }
        }
        var defaultID = AudioDeviceID(0)
        var defaultAddress = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDefaultInputDevice,
            mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        var defaultSize = UInt32(MemoryLayout<AudioDeviceID>.size)
        let hasDefault = AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &defaultAddress,
            0, nil, &defaultSize, &defaultID) == noErr
        return ["input": input, "output": output,
                "default_input_uid": hasDefault ? (stringProperty(defaultID, selector: kAudioDevicePropertyDeviceUID) ?? "") : ""]
    }

    private func audioDeviceID(forUID uid: String) -> AudioDeviceID? {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyDevices,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size) == noErr else {
            return nil
        }
        let count = Int(size) / MemoryLayout<AudioDeviceID>.size
        var devices = [AudioDeviceID](repeating: 0, count: count)
        guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &devices) == noErr else {
            return nil
        }
        for device in devices where Self.stringProperty(device, selector: kAudioDevicePropertyDeviceUID) == uid {
            return device
        }
        return nil
    }

    private func configureInputDevice(_ input: AVAudioInputNode) -> String? {
        guard let uid = selectedDeviceUID?.trimmingCharacters(in: .whitespacesAndNewlines),
              !uid.isEmpty else { return nil }
        guard let deviceID = audioDeviceID(forUID: uid) else {
            return "mic_device_not_found: \(uid)"
        }
        guard let audioUnit = input.audioUnit else {
            return "mic_input_audio_unit_unavailable"
        }
        var selectedID = deviceID
        let status = AudioUnitSetProperty(
            audioUnit,
            kAudioOutputUnitProperty_CurrentDevice,
            kAudioUnitScope_Global,
            0,
            &selectedID,
            UInt32(MemoryLayout<AudioDeviceID>.size)
        )
        if status == noErr {
            log("🎤 Mic input device selected: \(uid)")
            return nil
        } else {
            return "mic_device_selection_failed: \(status)"
        }
    }

    /// Called once at daemon startup: just enough to verify TCC permission, then stop.
    /// Avoids displaying the macOS microphone-in-use indicator while the daemon is idle.
    func probePermission() {
        let engine = AVAudioEngine()
        _ = engine.inputNode  // touching this triggers the TCC prompt
        do {
            try engine.start()
            engine.stop()
            MIC_READY = true; MIC_ERROR = ""; log("🎤 Mic probe OK (idle until recording starts)")
        } catch {
            MIC_READY = false; MIC_ERROR = "\(error)"; log("Mic probe failed: \(error)")
        }
    }

    func start() {
        lifecycleQueue.sync { startOnQueue() }
    }

    private func startOnQueue() {
        guard engine == nil else { return }
        let engine = AVAudioEngine()
        let input = engine.inputNode
        if let error = configureInputDevice(input) {
            MIC_READY = false
            MIC_ERROR = error
            log("🎤 Mic input device configuration failed: \(error)")
            return
        }
        let fmt = input.outputFormat(forBus: 0)

        input.installTap(onBus: 0, bufferSize: 4096, format: fmt) { [weak self] buf, _ in
            guard let self = self else { return }
            guard let chData = buf.floatChannelData else { return }
            let len = Int(buf.frameLength)
            let channels = max(1, Int(buf.format.channelCount))
            let samples: [Float]
            if channels == 1 {
                samples = Array(UnsafeBufferPointer(start: chData[0], count: len))
            } else {
                var mixed = [Float](repeating: 0, count: len)
                for channel in 0..<channels {
                    let channelSamples = UnsafeBufferPointer(start: chData[channel], count: len)
                    for i in 0..<len {
                        mixed[i] += channelSamples[i]
                    }
                }
                let divisor = Float(channels)
                samples = mixed.map { $0 / divisor }
            }
            self.recorder.onMicAudio(samples, sourceRate: buf.format.sampleRate)
        }

        do {
            try engine.start()
            self.engine = engine
            generation &+= 1
            let currentGeneration = generation
            configurationObserver = NotificationCenter.default.addObserver(
                forName: .AVAudioEngineConfigurationChange,
                object: engine,
                queue: nil
            ) { [weak self, weak engine] _ in
                guard let self = self, let engine = engine else { return }
                self.lifecycleQueue.async {
                    guard self.engine === engine,
                          self.generation == currentGeneration else { return }
                    self.restartAfterConfigurationChange()
                }
            }
            MIC_READY = true; MIC_ERROR = ""
            log("🎤 Mic capture started (channels=\(fmt.channelCount), gain=\(recorder.micGain)x)")
        }
        catch { MIC_READY = false; MIC_ERROR = "\(error)"; log("Mic start failed: \(error)") }
    }

    private func restartAfterConfigurationChange() {
        log("🎤 Mic audio route changed — restarting capture")
        recorder.noteCaptureRouteChange()
        stopOnQueue(logIdle: false)
        Thread.sleep(forTimeInterval: 0.1)
        startOnQueue()
    }

    func stop() {
        lifecycleQueue.sync { stopOnQueue(logIdle: true) }
    }

    private func stopOnQueue(logIdle: Bool) {
        generation &+= 1
        if let observer = configurationObserver {
            NotificationCenter.default.removeObserver(observer)
            configurationObserver = nil
        }
        if let engine = engine {
            engine.inputNode.removeTap(onBus: 0)
            engine.stop()
            self.engine = nil
        }
        if logIdle { log("🎤 Mic idle") }
    }
}

// ─── SCStream 输出处理器 ──────────────────────────────

@available(macOS 12.3, *)
class SysAudioOutput: NSObject, SCStreamOutput {
    unowned let recorder: AudioRecorder
    init(_ r: AudioRecorder) { recorder = r }

    func stream(_ s: SCStream, didOutputSampleBuffer buf: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .audio else { return }
        let asbd = buf.formatDescription.flatMap { CMAudioFormatDescriptionGetStreamBasicDescription($0)?.pointee }
        // BEGIN input-format sck-validate
        guard let actual = asbd, CapturePCMFormat(actual) != nil else {
            SYS_READY = false; SYS_ERROR = CaptureInputFormatFault.invalid.rawValue
            recorder.rejectSourceFormat(); return
        }
        if SYS_ERROR == CaptureInputFormatFault.invalid.rawValue {
            SYS_READY = true; SYS_ERROR = ""
        }
        // END input-format sck-validate
        let flags = asbd?.mFormatFlags ?? 0
        let channels = Int(asbd?.mChannelsPerFrame ?? 2)
        let isFloat = (flags & kAudioFormatFlagIsFloat) != 0
        let isInt = (flags & kAudioFormatFlagIsSignedInteger) != 0
        let nonInterleaved = (flags & kAudioFormatFlagIsNonInterleaved) != 0
        if !SYS_FORMAT_LOGGED, let a = asbd {
            SYS_FORMAT_LOGGED = true
            log("SC audio ASBD: sr=\(a.mSampleRate) ch=\(a.mChannelsPerFrame) bits=\(a.mBitsPerChannel) bytesFrame=\(a.mBytesPerFrame) bytesPacket=\(a.mBytesPerPacket) framesPacket=\(a.mFramesPerPacket) float=\(isFloat) int=\(isInt) nonInterleaved=\(nonInterleaved) flags=0x\(String(flags, radix:16))")
        }
        guard let db = buf.dataBuffer else { return }
        var ptr: UnsafeMutablePointer<Int8>?; var len: Int = 0
        guard CMBlockBufferGetDataPointer(db, atOffset: 0, lengthAtOffsetOut: &len, totalLengthOut: nil, dataPointerOut: &ptr) == noErr,
              let p = ptr, len > 0 else { return }
        let cnt = len / MemoryLayout<Float>.size
        guard cnt > 0 else { return }

        let raw = UnsafeMutableRawPointer(p)
        let floats = [Float](UnsafeBufferPointer(start: raw.assumingMemoryBound(to: Float.self), count: cnt))
        let stereoFloats: [Float]
        if isFloat && nonInterleaved && channels >= 2 {
            // ScreenCaptureKit reports planar Float32. In CMBlockBuffer this is
            // laid out as all left samples followed by all right samples.
            let frames = cnt / channels
            var interleaved = [Float](); interleaved.reserveCapacity(frames * 2)
            for i in 0..<frames {
                interleaved.append(floats[i])
                interleaved.append(floats[frames + i])
            }
            stereoFloats = interleaved
        } else if isFloat && channels == 1 {
            var stereo = [Float](); stereo.reserveCapacity(cnt * 2)
            for v in floats { stereo.append(v); stereo.append(v) }
            stereoFloats = stereo
        } else {
            stereoFloats = floats
        }
        guard !stereoFloats.isEmpty else { return }
        let int16s = stereoFloats.map { Int16(max(-1.0, min(1.0, $0)) * Float(Int16.max)) }
        recorder.onSysAudio(int16s, sourceRate: asbd?.mSampleRate ?? 0)
    }
}

// ─── 捕获后端协议 (CaptureBackend) ────────────────────
//
// PLAT-01 / D-02: the single platform seam for system-audio capture. It hides
// BOTH the ScreenCaptureKit vocabulary (SCStreamConfiguration / SCContentFilter)
// and the macOS-14.4 Core Audio process-tap vocabulary (CATapDescription) behind
// "emit PCM frames + list sources." The frame sink stays exactly as today —
// each conformer converts its native buffer to interleaved Int16 and pushes it
// through `recorder.onSysAudio([Int16])`.
//
// D-09 (interface neutrality): NO SCStreamConfiguration / SCContentFilter /
// CATapDescription / TCC token may appear in this protocol or in CaptureSource.
// The arms own those internally. The 13–14.3 arm is ScreenCaptureKitBackend
// (below); the 14.4+ ProcessTapBackend arm lands in 02-04 behind `if #available`.

/// A capturable audio source (display / app / system). Neutral by design:
/// SCK derives these from SCShareableContent displays; the tap arm will derive
/// them from the process-object list. Hides both representations.
struct CaptureSource {
    let id: String
    let name: String
    let kind: String   // "display" | "app" | "system"
}

protocol CaptureBackend: AnyObject {
    /// True once the backend has verified its capture permission (TCC handshake done).
    var isReady: Bool { get }
    /// Last capture error surfaced by the backend ("" when healthy).
    var lastError: String { get }

    /// Probe permission without leaving the OS recording indicator on (idle daemon).
    func probePermission()

    /// Begin emitting system-audio PCM to the sink. Blocks until actually capturing.
    func startCapture()
    /// Stop capturing; blocks until the OS-level capture indicator clears.
    func stopCapture()

    /// Capturable sources. SCK: SCShareableContent displays; taps: process list.
    func sources() -> [CaptureSource]
}

// ─── 屏幕捕获管理器 (ScreenCaptureKit arm, macOS 13–14.3) ─────
//
// D-03: this is the EXISTING capture code, refactored in place to conform to
// CaptureBackend — NOT rewritten. The planar-Float32 → interleaved-Int16
// conversion (SysAudioOutput, above) and the start/stop/probe bodies are kept
// verbatim; only the type name, conformance, and isReady/lastError/sources()
// bridge are added.

@available(macOS 12.3, *)
final class ScreenCaptureKitBackend: CaptureBackend {
    let recorder: AudioRecorder
    let output: SysAudioOutput
    var stream: SCStream?

    init(recorder: AudioRecorder) {
        self.recorder = recorder
        self.output = SysAudioOutput(recorder)
    }

    /// CaptureBackend.isReady — bridge the existing process-level SYS_READY global.
    var isReady: Bool { SYS_READY }
    /// CaptureBackend.lastError — bridge the existing process-level SYS_ERROR global.
    var lastError: String { SYS_ERROR }

    /// CaptureBackend.sources() — minimal source list from SCShareableContent
    /// displays. Returns [] if the shareable content cannot be fetched in time;
    /// callers treat an empty list as "no enumerable sources right now."
    func sources() -> [CaptureSource] {
        var result: [CaptureSource] = []
        let sem = DispatchSemaphore(value: 0)
        Task {
            defer { sem.signal() }
            if let content = try? await SCShareableContent.current {
                for d in content.displays {
                    result.append(CaptureSource(
                        id: String(d.displayID),
                        name: "Display \(d.displayID)",
                        kind: "display"
                    ))
                }
            }
        }
        _ = sem.wait(timeout: .now() + 2)
        return result
    }

    /// Called once at daemon startup: open an SCStream just long enough to verify the
    /// TCC permission, then stop it. This means the menu-bar "screen recording in
    /// progress" indicator does NOT light up while the daemon is idle — only when an
    /// actual recording is in flight.
    func probePermission() {
        Task {
            do {
                let content = try await SCShareableContent.current
                guard let d = content.displays.first else {
                    SYS_READY = false; SYS_ERROR = "no display"
                    log("Sys probe failed: no display"); return
                }
                let filter = SCContentFilter(display: d, excludingWindows: [])
                let config = SCStreamConfiguration()
                config.capturesAudio = true
                let s = SCStream(filter: filter, configuration: config, delegate: nil)
                try await s.startCapture()
                try? await s.stopCapture()  // immediately tear down — we just wanted the TCC handshake
                SYS_READY = true; SYS_ERROR = ""
                log("🔊 Sys capture probe OK (idle until recording starts)")
            } catch {
                SYS_READY = false; SYS_ERROR = (error as NSError).localizedDescription
                log("Sys capture probe failed: \(SYS_ERROR)")
            }
        }
    }

    /// Start capturing system audio. Blocks until the SCStream is actually
    /// running (or fails), so that a subsequent stopCapture() call can rely
    /// on `stream` being set. Without this, start/stop racing each other
    /// produced "Sys capture started" messages AFTER "Sys capture idle" —
    /// and worse, left the macOS recording indicator on after stop.
    func startCapture() {
        if SYS_DISABLED {
            log("🔇 SYS_DISABLED — mic-only recording mode")
            SYS_READY = false
            return
        }
        guard stream == nil else { return }  // already capturing — idempotent
        let sem = DispatchSemaphore(value: 0)
        Task {
            defer { sem.signal() }
            do {
                let content = try await SCShareableContent.current
                guard let d = content.displays.first else { log("No display"); return }
                let filter = SCContentFilter(display: d, excludingWindows: [])
                let config = SCStreamConfiguration()
                config.capturesAudio = true
                let s = SCStream(filter: filter, configuration: config, delegate: nil)
                try await s.startCapture()
                try s.addStreamOutput(output, type: .audio, sampleHandlerQueue: .global())
                self.stream = s
                SYS_READY = true; SYS_ERROR = ""
                log("🔊 Sys capture started (display \(d.displayID))")
            } catch {
                SYS_READY = false; SYS_ERROR = (error as NSError).localizedDescription
                log("Sys capture failed: \(SYS_ERROR)")
            }
        }
        _ = sem.wait(timeout: .now() + 5)  // 5s upper bound for SCStream init
    }

    /// Stop capturing. Blocks until the SCStream has actually stopped, so
    /// the macOS recording indicator clears synchronously with the user's
    /// "stop" command.
    func stopCapture() {
        let s = stream
        stream = nil
        let sem = DispatchSemaphore(value: 0)
        Task {
            defer { sem.signal() }
            try? await s?.stopCapture()
        }
        _ = sem.wait(timeout: .now() + 2)
        log("🔇 Sys capture idle")
    }
}

// ─── 进程 Tap 捕获 (Core Audio process-tap arm, macOS 14.4+) ─────
//
// PLAT-02 / D-01 / D-03: the 14.4+ system-audio arm. On 14.4+ the AppDelegate
// selects this behind `if #available(macOS 14.4, *)`; 13–14.3 stays on
// ScreenCaptureKitBackend (the `else`). The macOS FLOOR is NOT raised — this
// arm is gated at 14.4 ONLY because the underlying Core Audio process-tap +
// aggregate-device APIs are unreliable below it (symbols exist at 14.2, but the
// AudioCap project pins stable behavior to 14.4). Do NOT lower the gate.
//
// Why a tap instead of ScreenCaptureKit: the tap requests the narrower
// "System Audio Recording Only" TCC scope, so 14.4+ users escape the weekly
// "Screen & System Audio Recording" re-permission nag (success criterion 3).
//
// Capture sequence (VERIFIED API names, 02-RESEARCH.md:342-405):
//   CATapDescription(stereoGlobalTapButExcludeProcesses: [])  → whole-system tap
//   → AudioHardwareCreateProcessTap → read the tap UID
//   → AudioHardwareCreateAggregateDevice (TapList = [{SubTapUID: <uid>}])
//   → AudioDeviceCreateIOProcIDWithBlock → AudioDeviceStart
// The IO callback delivers Float32 frames; they are converted to interleaved
// Int16 using the SAME clamp/interleave as SysAudioOutput (no re-derivation)
// and pushed through recorder.onSysAudio([Int16]) — the identical frame sink
// the SCK arm uses.
//
// Pitfall 3 (VERIFIED Apple bug, 02-RESEARCH.md:322-326): the tap can keep
// firing the IO callback with correct frameCounts yet deliver all-zero samples
// (silent recording while audio is audible), triggered by sample-rate
// renegotiation, Bluetooth sleep/wake, or long uptime. This backend detects a
// window of frameCount>0-yet-all-zero buffers and recovers by tearing the whole
// tap+aggregate stack down and rebuilding it (it does NOT merely log). The
// existing silence-monitor (startSilenceMonitor) only auto-stops when BOTH
// channels are quiet, so a sys-only zero-out does not false-stop the recording.

private struct ZeroBufferRecoveryPolicy {
    let threshold: Int
    let maxAttempts: Int
    private(set) var consecutiveZeroCallbacks = 0
    private(set) var attempts = 0
    private(set) var episode: UInt64 = 0
    private var armed = true

    init(threshold: Int, maxAttempts: Int) {
        self.threshold = threshold
        self.maxAttempts = maxAttempts
    }

    mutating func resetForRecording() {
        consecutiveZeroCallbacks = 0
        attempts = 0
        episode &+= 1
        armed = true
    }

    mutating func commitRecoveryAttempt() {
        attempts += 1
        consecutiveZeroCallbacks = 0
    }

    mutating func rearmAfterRecovery() {
        consecutiveZeroCallbacks = 0
        episode &+= 1
        armed = true
    }

    mutating func observe(allZero: Bool, running: Bool, rebuilding: Bool) -> Bool {
        guard running else {
            consecutiveZeroCallbacks = 0
            return false
        }
        guard allZero else {
            if consecutiveZeroCallbacks > 0 || !armed { episode &+= 1 }
            consecutiveZeroCallbacks = 0
            armed = true
            return false
        }
        if consecutiveZeroCallbacks == 0 { episode &+= 1 }
        consecutiveZeroCallbacks += 1
        guard armed, !rebuilding, attempts < maxAttempts,
              consecutiveZeroCallbacks >= threshold else { return false }
        armed = false
        return true
    }
}

@available(macOS 14.4, *)
final class ProcessTapBackend: CaptureBackend {
    let recorder: AudioRecorder

    private var tapID: AudioObjectID = 0
    private var aggID: AudioObjectID = 0
    private var ioProcID: AudioDeviceIOProcID?
    // BEGIN input-format state
    private let inputFormats = TapInputFormatLifecycle()
    private let formatControlQueue = DispatchQueue(label: "org.openmeetingmemo.tap-format")
    private var formatListeners: [(AudioObjectID, AudioObjectPropertyAddress, AudioObjectPropertyListenerBlock)] = []
    // END input-format state

    private let lock = NSLock()
    private let lifecycleLock = NSLock()
    private var running = false
    private var _lastError = ""

    // ── Pitfall 3 zero-buffer detection state ──
    // Real silence and a stuck tap both arrive as all-zero buffers. Recover once
    // per continuous zero-buffer episode, re-arm only after real audio returns,
    // and cap the total attempts so a quiet recording cannot rebuild forever.
    private var zeroRecoveryPolicy = ZeroBufferRecoveryPolicy(
        threshold: 200,
        maxAttempts: 3
    )
    private var rebuilding = false
    private var captureGeneration: UInt64 = 0

    init(recorder: AudioRecorder) {
        self.recorder = recorder
    }

    /// CaptureBackend.isReady — bridge to the process-level SYS_READY global so
    /// the socket "status"/"start" gating treats the tap exactly like the SCK arm.
    var isReady: Bool { SYS_READY }
    /// CaptureBackend.lastError — bridge the process-level SYS_ERROR global.
    var lastError: String { SYS_ERROR }

    /// CaptureBackend.sources() — the tap captures the whole system, so it
    /// exposes a single neutral "system" source rather than a per-display list.
    func sources() -> [CaptureSource] {
        return [CaptureSource(id: "system", name: "System Audio", kind: "system")]
    }

    /// Probe permission without leaving a recording indicator on. Building and
    /// immediately tearing down the tap forces the one-time
    /// "System Audio Recording Only" TCC handshake (NSAudioCaptureUsageDescription
    /// prompt) so SYS_READY is accurate while the daemon is idle.
    func probePermission() {
        lifecycleLock.lock()
        let ok = buildTap(probe: true)
        teardown()
        lifecycleLock.unlock()
        if ok {
            SYS_READY = true; SYS_ERROR = ""
            log("🔊 Sys tap probe OK (idle until recording starts)")
        } else {
            SYS_READY = false; SYS_ERROR = _lastError
            log("Sys tap probe failed: \(_lastError)")
        }
    }

    /// Start capturing system audio via the process tap. Blocks until the
    /// aggregate device's IO proc is started (or fails), matching the SCK arm's
    /// synchronous start/stop contract.
    func startCapture() {
        if SYS_DISABLED {
            log("🔇 SYS_DISABLED — mic-only recording mode")
            SYS_READY = false
            return
        }
        lock.lock()
        let already = running
        lock.unlock()
        guard !already else { return }   // idempotent — already capturing

        // Defensive: a prior probe / failed build / stale recording may have left
        // a half-open tap+aggregate (non-zero ids while running == false). Tear it
        // down before rebuilding so a re-arm never stacks a second tap or inherits
        // a poisoned one. teardown() is idempotent on zeroed ids, so this is a
        // no-op in the common clean-start case.
        lifecycleLock.lock()
        teardown()

        if buildTap(probe: false) {
            lock.lock()
            running = true
            rebuilding = false
            captureGeneration &+= 1
            zeroRecoveryPolicy.resetForRecording()
            lock.unlock()
            SYS_READY = true; SYS_ERROR = ""
            lifecycleLock.unlock()
            log("🔊 Sys tap capture started")
        } else {
            lifecycleLock.unlock()
            SYS_READY = false; SYS_ERROR = _lastError
            log("Sys tap capture failed: \(_lastError)")
        }
    }

    /// Stop capturing; tears the whole tap+aggregate stack down. Blocks until the
    /// OS-level device is stopped so the recording indicator clears synchronously.
    func stopCapture() {
        lock.lock(); running = false; lock.unlock()
        lifecycleLock.lock()
        teardown()
        lifecycleLock.unlock()
        log("🔇 Sys tap idle")
    }

    // ── Tap construction (02-RESEARCH.md:342-405) ──
    //
    // Builds the global tap, the private aggregate device wrapping it, and the IO
    // proc. Returns true once AudioDeviceStart succeeds. On any failure it records
    // `_lastError`, tears down whatever was partially created, and returns false.
    private func buildTap(probe: Bool) -> Bool {
        // 1. Describe a whole-system tap (empty exclude list = all processes).
        let desc = CATapDescription(stereoGlobalTapButExcludeProcesses: [])
        desc.isPrivate = true            // don't expose the tap device system-wide
        desc.muteBehavior = .unmuted     // passthrough: the user still hears the meeting
        desc.name = "OMM-SysTap"

        // 2. Create the tap → an AudioObjectID.
        var tap: AudioObjectID = 0
        let tapErr = AudioHardwareCreateProcessTap(desc, &tap)
        guard tapErr == noErr, tap != 0 else {
            logEvent(.tapCreateError, code: tapErr)
            _lastError = "AudioHardwareCreateProcessTap failed (OSStatus \(tapErr))"
            return false
        }
        logEvent(.tapCreateSuccess, code: probe ? 1 : 0)
        tapID = tap

        // 3. Read the tap's UID string (needed as the aggregate's SubTap UID).
        guard let tapUID = readTapUID(tap) else {
            _lastError = "tap UID read failed"
            teardown()
            return false
        }

        // 4. Build a PRIVATE aggregate device that contains the tap.
        //    IsPrivate keeps it out of the user-visible device list (T-02-14).
        let aggDict: [String: Any] = [
            kAudioAggregateDeviceNameKey as String:         "OMM-SysTap",
            kAudioAggregateDeviceUIDKey as String:          UUID().uuidString,
            kAudioAggregateDeviceIsPrivateKey as String:    true,
            kAudioAggregateDeviceIsStackedKey as String:    false,
            kAudioAggregateDeviceTapAutoStartKey as String: true,
            kAudioAggregateDeviceTapListKey as String: [
                [ kAudioSubTapUIDKey as String: tapUID ],
            ],
        ]
        var agg: AudioObjectID = 0
        let aggErr = AudioHardwareCreateAggregateDevice(aggDict as CFDictionary, &agg)
        guard aggErr == noErr, agg != 0 else {
            logEvent(.aggregateCreateError, code: aggErr)
            _lastError = "AudioHardwareCreateAggregateDevice failed (OSStatus \(aggErr))"
            teardown()
            return false
        }
        aggID = agg

        // BEGIN input-format validate-build
        guard let context = prepareInputFormat({ self.readInputFormat(agg) }) else {
            teardown(); return false
        }
        // END input-format validate-build
        // A bare permission probe only needs the tap+aggregate to have been
        // accepted by the HAL (that is what forces the TCC handshake). Skip the
        // IO proc so the probe never streams a single frame.
        if probe { return publishInputFormatReady(context) }
        // BEGIN input-format listeners-build
        guard installFormatListeners(context) else { teardown(); return false }
        // END input-format listeners-build

        // 5. Install an IO proc that converts Float32 → interleaved Int16 and feeds
        //    the existing sink. The block runs on a realtime CoreAudio thread.
        var proc: AudioDeviceIOProcID?
        let procErr = AudioDeviceCreateIOProcIDWithBlock(&proc, agg, nil) {
            [weak self, context] (_, inInputData, _, _, _) in
            guard let self = self else { return }
            self.handleIO(inInputData, context: context)
        }
        guard procErr == noErr, let p = proc else {
            logEvent(.ioProcCreateError, code: procErr)
            _lastError = "AudioDeviceCreateIOProcIDWithBlock failed (OSStatus \(procErr))"
            teardown()
            return false
        }
        ioProcID = p

        let startErr = AudioDeviceStart(agg, p)
        guard startErr == noErr else {
            logEvent(.deviceStartError, code: startErr)
            _lastError = "AudioDeviceStart failed (OSStatus \(startErr))"
            teardown()
            return false
        }
        return publishInputFormatReady(context)
    }

    /// Read kAudioTapPropertyUID off the tap object → its CFString UID.
    private func readTapUID(_ tap: AudioObjectID) -> String? {
        var addr = AudioObjectPropertyAddress(
            mSelector: kAudioTapPropertyUID,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var uidCF: CFString = "" as CFString
        var size = UInt32(MemoryLayout<CFString>.size)
        let err = withUnsafeMutablePointer(to: &uidCF) { ptr -> OSStatus in
            AudioObjectGetPropertyData(tap, &addr, 0, nil, &size, ptr)
        }
        guard err == noErr else { return nil }
        return uidCF as String
    }

    // BEGIN input-format lifecycle-test
    func inputFormatLifecycleSelfTest() {
        let directory = RECORDING_DIR.appendingPathComponent("format-lifecycle")
        try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        recorder.configure(silenceSeconds: 3600, silenceThreshold: 0, outputDir: directory)
        assert(saveBridge(["session_id": UUID().uuidString.lowercased(), "origin_instance_id": SERVICE_OWNER, "files": [], "recording": true]))
        assert(recorder.start(title: "Synthetic") != nil)
        var queries = 0
        func description(_ rate: Double) -> AudioStreamBasicDescription {
            let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: rate, channels: 2, interleaved: true)!
            return withExtendedLifetime(format) { format.streamDescription.pointee }
        }
        func prepare(_ rate: Double) -> TapIOFormatContext {
            formatControlQueue.sync {
                let value = prepareInputFormat { queries += 1; return .success(description(rate)) }!
                assert(!inputFormats.ready)
                assert(publishInputFormatReady(value)) // injected successful device activation; no native device is created
                assert(SYS_READY && SYS_ERROR.isEmpty)
                return value
            }
        }
        func feed(_ context: TapIOFormatContext, count: Int) {
            let frames = Int(context.format.rate / 100)
            let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: context.format.rate, channels: 2, interleaved: true)!
            let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(frames))!
            buffer.frameLength = AVAudioFrameCount(frames)
            let values = buffer.mutableAudioBufferList.pointee.mBuffers.mData!.assumingMemoryBound(to: Float.self)
            for i in 0..<(frames * 2) { values[i] = 0.125 }
            let before = queries
            for _ in 0..<count { handleIO(buffer.audioBufferList, context: context) }
            assert(queries == before, "IO callback must never call the format provider")
        }
        func notifyFormatChange(_ context: TapIOFormatContext) {
            formatControlQueue.sync {
                var address = AudioObjectPropertyAddress(mSelector: kAudioTapPropertyFormat, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
                withUnsafePointer(to: &address) { inputFormatListener(context)(1, $0) }
            }
        }
        var old: TapIOFormatContext? = prepare(44100)
        weak var retainedOld = old
        var queuedOld: (() -> Void)? = { [old = old!] in feed(old, count: 1) }
        feed(old!, count: 100)
        notifyFormatChange(old!)
        assert(!SYS_READY && SYS_ERROR == CaptureInputFormatFault.changed.rawValue && !inputFormats.ready)
        var current = prepare(48000)
        assert(!old!.isActive && !inputFormats.didStart(old!))
        let before = recorder.captureClock
        queuedOld!() // callback retained across destroy: inactive generation cannot enqueue stale format
        assert((recorder.captureClock["system"] as! [String: Any])["input_frames"] as! UInt64 == (before["system"] as! [String: Any])["input_frames"] as! UInt64)
        old = nil; assert(retainedOld != nil); queuedOld = nil; assert(retainedOld == nil)
        feed(current, count: 1000)
        notifyFormatChange(current)
        assert(!SYS_READY && !current.isActive)
        current = prepare(44100); feed(current, count: 100)
        assert(queries == 3)
        formatControlQueue.sync {
            SYS_READY = true; SYS_ERROR = ""
            assert(prepareInputFormat { queries += 1; return .failure(.unavailable) } == nil)
            assert(!SYS_READY && SYS_ERROR == CaptureInputFormatFault.unavailable.rawValue && !inputFormats.ready)
            assert(CaptureInputFormatFault.startError(SYS_ERROR) == CaptureInputFormatFault.unavailable.rawValue)
            assert(CaptureInputFormatFault.startError("legacy permission failure") == "sys_capture_not_ready")
            assert(CaptureInputFormatFault.stopError(SYS_ERROR, clock: [:]) == CaptureInputFormatFault.unavailable.rawValue)
            assert(CaptureInputFormatFault.stopError(SYS_ERROR, clock: recorder.captureClock) == nil)
            for field in 0..<4 {
                var invalid = description(44100)
                if field == 0 { invalid.mSampleRate = .nan }
                if field == 1 { invalid.mChannelsPerFrame = 3 }
                if field == 2 { invalid.mBitsPerChannel = 16 }
                if field == 3 { invalid.mBytesPerFrame = 1 }
                assert(prepareInputFormat { queries += 1; return .success(invalid) } == nil)
                assert(!SYS_READY && SYS_ERROR == CaptureInputFormatFault.invalid.rawValue && !inputFormats.ready)
            }
        }
        current = prepare(48000); feed(current, count: 100)
        // A malformed actual IO buffer takes the production off-RT fault path.
        var malformed = AudioBufferList(mNumberBuffers: 1, mBuffers: AudioBuffer(mNumberChannels: 1, mDataByteSize: 0, mData: nil))
        withUnsafePointer(to: &malformed) { handleIO($0, context: current) }
        formatControlQueue.sync {} // wait only in this non-RT synthetic test
        assert(!current.isActive && !SYS_READY && SYS_ERROR == CaptureInputFormatFault.buffer.rawValue)
        current = prepare(44100); feed(current, count: 100)
        let clock = recorder.captureClock
        let system = clock["system"] as! [String: Any]
        assert(system["input_frames"] as! UInt64 > 0 && system["target_frames"] as! UInt64 > 0)
        assert(system["rate_changes"] as! Int >= 3)
        assert(clock["format_rejects"] as! UInt64 >= 6)
        assert(queries == 10)
        _ = recorder.stop()
        assert(recorder.start(title: "SyntheticEmpty") != nil)
        // A stop may race the off-RT rebuild. Hold that queue and verify the IO fault is already observable.
        let entered = DispatchSemaphore(value: 0), release = DispatchSemaphore(value: 0)
        formatControlQueue.async { entered.signal(); _ = release.wait(timeout: .now() + 5) }
        assert(entered.wait(timeout: .now() + 5) == .success)
        withUnsafePointer(to: &malformed) { handleIO($0, context: current) }
        _ = recorder.stop()
        assert(CaptureInputFormatFault.stopError("", clock: recorder.captureClock) == CaptureInputFormatFault.buffer.rawValue)
        release.signal()
        formatControlQueue.sync { teardown() }
        assert(!current.isActive)
        print("input-format-lifecycle-self-test passed; provider_calls=10; actual_IO_callbacks=1400; route_listener_callbacks=2; stale_context_rejected=true; query_failure_and_invalid_format_not_ready=true; safe_recovery=true")
    }
    // END input-format lifecycle-test

    // BEGIN input-format helpers
    private func publishInputFormatReady(_ context: TapIOFormatContext) -> Bool {
        guard inputFormats.didStart(context) else {
            _lastError = CaptureInputFormatFault.buffer.rawValue
            SYS_READY = false; SYS_ERROR = _lastError
            teardown(); return false
        }
        SYS_READY = true; SYS_ERROR = ""; _lastError = ""
        return true
    }

    private func readInputFormat(_ aggregate: AudioObjectID) -> Result<AudioStreamBasicDescription, CaptureInputFormatFault> {
        // Actual aggregate input stream ASBD, never the device's nominal clock.
        var address = AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyStreamFormat,
            mScope: kAudioDevicePropertyScopeInput, mElement: kAudioObjectPropertyElementMain)
        var description = AudioStreamBasicDescription()
        var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
        guard AudioObjectGetPropertyData(aggregate, &address, 0, nil, &size, &description) == noErr,
              size == UInt32(MemoryLayout<AudioStreamBasicDescription>.size) else { return .failure(.unavailable) }
        return .success(description)
    }

    private func prepareInputFormat(_ provider: () -> Result<AudioStreamBasicDescription, CaptureInputFormatFault>) -> TapIOFormatContext? {
        guard let context = inputFormats.prepare(provider) else {
            _lastError = (inputFormats.fault ?? .invalid).rawValue
            SYS_READY = false; SYS_ERROR = _lastError
            recorder.rejectSourceFormat()
            return nil
        }
        return context
    }

    private func inputFormatListener(_ context: TapIOFormatContext) -> AudioObjectPropertyListenerBlock {
        { [weak self, context] _, _ in
            context.invalidate()
            self?.rebuildInputFormat(context, reason: .changed)
        }
    }

    private func installFormatListeners(_ context: TapIOFormatContext) -> Bool {
        for (object, selector, scope) in [
            (tapID, kAudioTapPropertyFormat, kAudioObjectPropertyScopeGlobal),
            (aggID, kAudioDevicePropertyStreamFormat, kAudioDevicePropertyScopeInput)
        ] {
            var address = AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
            let listener = inputFormatListener(context)
            guard AudioObjectAddPropertyListenerBlock(object, &address, formatControlQueue, listener) == noErr else {
                _lastError = CaptureInputFormatFault.unavailable.rawValue
                return false
            }
            formatListeners.append((object, address, listener))
        }
        return true
    }

    private func rejectInputFormat(_ context: TapIOFormatContext, reason: CaptureInputFormatFault) {
        guard context.invalidate() else { return }
        // Enqueue the numeric fault before a concurrent stop; never wait for the control queue from IO.
        recorder.rejectSourceFormat()
        formatControlQueue.async { [weak self, context] in self?.rebuildInputFormat(context, reason: reason) }
    }

    private func rebuildInputFormat(_ context: TapIOFormatContext, reason: CaptureInputFormatFault) {
        lifecycleLock.lock()
        defer { lifecycleLock.unlock() }
        guard inputFormats.current === context else { return }
        SYS_READY = false; SYS_ERROR = reason.rawValue; _lastError = reason.rawValue
        if reason != .buffer { recorder.rejectSourceFormat() }
        lock.lock(); let restart = running; rebuilding = restart; lock.unlock()
        teardown() // deactivate before stop/destroy; queued old blocks retain only an inactive context
        guard restart else { return }
        var rebuilt = false
        for delay in [0.0, 0.1, 0.3] {
            if delay > 0 { Thread.sleep(forTimeInterval: delay) }
            lock.lock(); let current = running; lock.unlock()
            guard current else { break }
            if buildTap(probe: false) { rebuilt = true; break }
        }
        lock.lock()
        let current = running
        rebuilding = false
        if rebuilt && current { zeroRecoveryPolicy.rearmAfterRecovery() }
        lock.unlock()
        if rebuilt && current { SYS_READY = true; SYS_ERROR = "" }
        else {
            teardown()
            SYS_READY = false; SYS_ERROR = _lastError
        }
    }
    // END input-format helpers

    // ── IO callback: Float32 AudioBufferList → interleaved Int16 → sink ──
    private func handleIO(_ inInputData: UnsafePointer<AudioBufferList>, context: TapIOFormatContext) {
        // The process tap's format, not the requested WAV clock, is bound to this IOProc generation.
        guard context.isActive else { return }
        let sourceRate = context.format.rate
        let abl = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: inInputData))
        // BEGIN input-format context-buffer
        guard context.format.accepts(abl) else {
            rejectInputFormat(context, reason: .buffer); return
        }
        // END input-format context-buffer
        guard abl.count > 0 else { return }

        // Gather Float32 samples across the buffer list. A tap aggregate may
        // deliver either one interleaved buffer (mChannelsPerFrame > 1) or
        // several mono buffers (non-interleaved). Normalize to interleaved
        // stereo Float32 first, then reuse the SysAudioOutput clamp verbatim.
        var stereoFloats: [Float] = []

        if abl.count == 1 {
            let buf = abl[0]
            let channels = Int(buf.mNumberChannels)
            let cnt = Int(buf.mDataByteSize) / MemoryLayout<Float>.size
            guard cnt > 0, let data = buf.mData else { return }
            let floats = [Float](UnsafeBufferPointer(
                start: data.assumingMemoryBound(to: Float.self), count: cnt))
            if channels >= 2 {
                // already interleaved L/R/L/R…
                stereoFloats = floats
            } else {
                // mono → duplicate into stereo
                stereoFloats.reserveCapacity(cnt * 2)
                for v in floats { stereoFloats.append(v); stereoFloats.append(v) }
            }
        } else {
            // Non-interleaved: buffer 0 = L, buffer 1 = R (extra channels ignored).
            let lBuf = abl[0]
            let rBuf = abl[1]
            let lCnt = Int(lBuf.mDataByteSize) / MemoryLayout<Float>.size
            let rCnt = Int(rBuf.mDataByteSize) / MemoryLayout<Float>.size
            let frames = min(lCnt, rCnt)
            guard frames > 0, let lData = lBuf.mData, let rData = rBuf.mData else { return }
            let lFloats = [Float](UnsafeBufferPointer(
                start: lData.assumingMemoryBound(to: Float.self), count: lCnt))
            let rFloats = [Float](UnsafeBufferPointer(
                start: rData.assumingMemoryBound(to: Float.self), count: rCnt))
            stereoFloats.reserveCapacity(frames * 2)
            for i in 0..<frames {
                stereoFloats.append(lFloats[i])
                stereoFloats.append(rFloats[i])
            }
        }

        guard !stereoFloats.isEmpty else { return }

        // Pitfall 3: detect frameCount>0-yet-all-zero. `allSatisfy { $0 == 0 }`
        // over a non-empty buffer means the tap claimed frames but delivered
        // silence — count the run and recover if it persists.
        let allZero = stereoFloats.allSatisfy { $0 == 0.0 }
        if allZero {
            lock.lock()
            let isRunning = running
            let tripped = zeroRecoveryPolicy.observe(
                allZero: true,
                running: isRunning,
                rebuilding: rebuilding
            )
            let currentZeroRun = zeroRecoveryPolicy.consecutiveZeroCallbacks
            let generation = captureGeneration
            let zeroEpisode = zeroRecoveryPolicy.episode
            if tripped {
                rebuilding = true
            }
            lock.unlock()
            if tripped {
                logEvent(.zeroRecovery, code: Int32(clamping: currentZeroRun))
                // Rebuild off the realtime IO thread.
                DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                    self?.recoverFromZeroBuffers(
                        generation: generation,
                        zeroEpisode: zeroEpisode
                    )
                }
            }
            return   // don't push silence into the WAV
        }

        // Real audio resets and re-arms the current zero-buffer episode.
        lock.lock()
        _ = zeroRecoveryPolicy.observe(
            allZero: false,
            running: running,
            rebuilding: rebuilding
        )
        lock.unlock()
        let int16s = stereoFloats.map { Int16(max(-1.0, min(1.0, $0)) * Float(Int16.max)) }
        recorder.onSysAudio(int16s, sourceRate: sourceRate)
    }

    /// Pitfall 3 recovery: full teardown then rebuild of the tap+aggregate stack.
    private func recoverFromZeroBuffers(generation: UInt64, zeroEpisode: UInt64) {
        lifecycleLock.lock()
        defer { lifecycleLock.unlock() }
        lock.lock()
        let sameGeneration = captureGeneration == generation
        let shouldRestart = running && sameGeneration
            && zeroRecoveryPolicy.episode == zeroEpisode
        if shouldRestart {
            zeroRecoveryPolicy.commitRecoveryAttempt()
        } else if sameGeneration {
            rebuilding = false
        }
        lock.unlock()
        guard shouldRestart else { return }

        // A delayed recovery from a previous recording must never tear down the
        // current recording's tap.
        teardown()
        var rebuilt = false
        for delay in [0.0, 0.1, 0.3] {
            if delay > 0 { Thread.sleep(forTimeInterval: delay) }
            lock.lock()
            let stillCurrent = running && captureGeneration == generation
            lock.unlock()
            guard stillCurrent else { break }
            if buildTap(probe: false) {
                rebuilt = true
                break
            }
        }
        if rebuilt {
            SYS_READY = true; SYS_ERROR = ""
            logEvent(.zeroRecoverySuccess)
        } else {
            SYS_READY = false; SYS_ERROR = _lastError
            logEvent(.zeroRecoveryFailure)
        }
        lock.lock()
        if captureGeneration == generation {
            rebuilding = false
            if rebuilt {
                zeroRecoveryPolicy.rearmAfterRecovery()
            } else {
                running = false
            }
        }
        lock.unlock()
    }

    /// Destroy the IO proc, aggregate device, and tap in the exact order from
    /// 02-RESEARCH.md:399-403. Safe to call repeatedly (idempotent on zeroed ids)
    /// and from both stopCapture() and the Pitfall-3 rebuild path.
    private func teardown() {
        // BEGIN input-format teardown
        inputFormats.invalidate()
        for (object, var address, listener) in formatListeners {
            AudioObjectRemovePropertyListenerBlock(object, &address, formatControlQueue, listener)
        }
        formatListeners.removeAll()
        // END input-format teardown
        if let proc = ioProcID {
            if aggID != 0 {
                AudioDeviceStop(aggID, proc)
                AudioDeviceDestroyIOProcID(aggID, proc)
            }
            ioProcID = nil
        }
        if aggID != 0 {
            AudioHardwareDestroyAggregateDevice(aggID)
            aggID = 0
        }
        if tapID != 0 {
            AudioHardwareDestroyProcessTap(tapID)
            tapID = 0
        }
    }
}

// ─── Socket 服务器 ────────────────────────────────────

extension AnchoredRecordingDirectory {
    func openExistingFile(named filename: String, writable: Bool = false) -> FileHandle? {
        guard !filename.isEmpty, filename != ".", filename != "..", !filename.contains("/"), stillAnchored() else { return nil }
        let fd = filename.withCString { Darwin.openat(targetFD, $0, (writable ? O_RDWR : O_RDONLY) | O_NOFOLLOW) }
        guard fd >= 0 else { return nil }
        var opened = stat(); var named = stat()
        let located = filename.withCString { Darwin.fstatat(targetFD, $0, &named, AT_SYMLINK_NOFOLLOW) }
        guard Darwin.fstat(fd, &opened) == 0, located == 0,
              (opened.st_mode & S_IFMT) == S_IFREG, opened.st_uid == geteuid(),
              opened.st_mode & 0o777 == 0o600, opened.st_nlink == 1,
              opened.st_dev == named.st_dev, opened.st_ino == named.st_ino, stillAnchored() else {
            Darwin.close(fd); return nil
        }
        return FileHandle(fileDescriptor: fd, closeOnDealloc: true)
    }

    // Repair only the owned interrupted file. PCM bytes and the imported WAV header layout stay unchanged.
    func sealInterruptedWav(named filename: String, legacy: Bool = false) -> Int? {
        guard let handle = openExistingFile(named: filename, writable: true) else { return nil }
        defer { try? handle.close() }
        do {
            let size = try handle.seekToEnd()
            guard size >= 82, size - 82 <= (legacy ? UInt64(UInt32.max) - 74 : UInt64(1800 * 48000 * 4)) else { return nil }
            try handle.seek(toOffset: 0)
            guard let data = try handle.read(upToCount: 82), data.count == 82 else { return nil }
            let h = [UInt8](data)
            guard Array(h[0..<4]) == Array("RIFF".utf8), Array(h[8..<12]) == Array("WAVE".utf8),
                  Array(h[12..<16]) == Array("fmt ".utf8), h[16] == 16,
                  h[17...19].allSatisfy({ $0 == 0 }), h[20] == 1, h[21] == 0,
                  h[22] == 2, h[23] == 0, Array(h[24..<28]) == [0x80, 0xbb, 0, 0],
                  h[34] == 16, h[35] == 0, Array(h[74..<78]) == Array("data".utf8) else { return nil }
            let payload = (size - 82) / 4 * 4
            try handle.truncate(atOffset: payload + 82)
            var riffSize = UInt32(payload + 74).littleEndian
            var dataSize = UInt32(payload).littleEndian
            try handle.seek(toOffset: 4)
            try withUnsafeBytes(of: &riffSize) { try handle.write(contentsOf: Data($0)) }
            try handle.seek(toOffset: 78)
            try withUnsafeBytes(of: &dataSize) { try handle.write(contentsOf: Data($0)) }
            try handle.synchronize()
            guard stillAnchored() else { return nil }
            return Int(payload / 4)
        } catch { return nil }
    }

    func publishDescriptor(named filename: String, data: Data) -> Bool {
        let temporary = UUID().uuidString + ".descriptor.tmp"
        guard !filename.contains("/"), stillAnchored(), let file = createFile(named: temporary) else { return false }
        defer { removeFile(named: temporary) }
        do {
            try file.handle.write(contentsOf: data)
            try file.handle.synchronize()
            try file.handle.close()
            let renamed = temporary.withCString { source in filename.withCString { destination in
                Darwin.renameatx_np(targetFD, source, targetFD, destination, UInt32(RENAME_EXCL))
            }}
            return renamed == 0 && Darwin.fsync(targetFD) == 0 && stillAnchored()
        } catch { return false }
    }

}

// MIT Yulu rolling-writer seam: split already paired PCM, never restart capture.
final class RollingRecordingWriter {
    private let directory: AnchoredRecordingDirectory
    private let filename: String
    private let maxFrames: Int
    private var current: WavWriter?
    private let title: String
    var framePosition: Int { offset + frames }
    private var frames = 0
    private var offset = 0
    private var sequence = 0
    private(set) var failed = false
    var captureClock: (() -> [String: Any])?
    var url: URL { current?.url ?? directory.targetURL.appendingPathComponent(filename) }
    init?(directory: AnchoredRecordingDirectory, filename: String, maxFrames: Int,
          sequence: Int = 0, offsetFrames: Int = 0, title: String = "meeting") {
        guard sequence >= 0, offsetFrames >= 0, maxFrames > 0, maxFrames <= 1800 * 48000,
              let writer = WavWriter(directory: directory, filename: filename) else { return nil }
        self.directory = directory; self.filename = filename; self.maxFrames = maxFrames; self.title = title
        self.current = writer; self.sequence = sequence; self.offset = offsetFrames
    }
    static func writableFrames(offered: Int, current: Int, limit: Int) -> Int {
        min(offered, limit - current)
    }
    func append(_ data: Data) {
        guard !failed, data.count % 4 == 0 else { failed = true; return }
        var position = 0
        while position < data.count {
            if current == nil {
                let next = String(filename.dropLast(4)) + String(format: "-%04d.wav", sequence)
                guard let writer = WavWriter(directory: directory, filename: next) else { failed = true; return }
                current = writer
                guard appendBridgeFile(writer.url.path, sequence: sequence, offsetFrames: offset, segmentMs: maxFrames / 48),
                      writeState(recording: true, title: title, path: writer.url.path, sequence: sequence, offsetFrames: offset, segmentMs: maxFrames / 48) else {
                    failed = true; return
                }
            }
            let count = Self.writableFrames(offered: (data.count - position) / 4,
                                            current: frames, limit: maxFrames) * 4
            current?.append(data.subdata(in: position..<(position + count)))
            frames += count / 4; position += count
            if frames == maxFrames { seal() }
            if failed { return }
        }
    }
    private func seal() {
        guard let writer = current else { return }
        writer.finalize()
        if frames > 0 {
            let bridge = readBridge()
            let document: [String: Any] = ["schema_version": 1, "temporary": true,
                "session_id": bridge["session_id"] ?? "", "instance_id": bridge["origin_instance_id"] ?? SERVICE_OWNER,
                "sequence": sequence,
                "offset_frames": offset, "frame_count": frames, "sample_rate": 48000,
                "path": writer.url.path, "capture_clock": captureClock?() ?? [:]]
            guard let data = try? JSONSerialization.data(withJSONObject: document),
                  directory.publishDescriptor(named: writer.url.lastPathComponent + ".sealed.json", data: data) else {
                failed = true; bridgeError = "capture_recovery_state_failed"; return
            }
            offset += frames; sequence += 1
        } else {
            directory.removeFile(named: writer.url.lastPathComponent)
            var bridge = readBridge()
            bridge["files"] = (bridge["files"] as? [String] ?? []).filter { $0 != writer.url.path }
            saveBridge(bridge)
        }
        failed = false; frames = 0; current = nil
    }
    func finalize() { seal() }
}


func unixSocketIsReachable(at path: URL) -> Bool {
    guard FileManager.default.fileExists(atPath: path.path) else { return false }
    let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
    guard fd >= 0 else { return false }
    defer { close(fd) }

    var addr = sockaddr_un()
    addr.sun_family = sa_family_t(AF_UNIX)
    _ = path.path.withCString { strncpy(&addr.sun_path.0, $0, min(path.path.utf8.count, 103)) }
    return withUnsafePointer(to: &addr) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
            Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) == 0
        }
    }
}

func acquireExclusiveFileLock(at path: URL) -> Int32? {
    let fd = Darwin.open(path.path, O_CREAT | O_RDWR, 0o600)
    guard fd >= 0 else { return nil }
    guard flock(fd, LOCK_EX | LOCK_NB) == 0 else {
        close(fd)
        return nil
    }
    return fd
}

enum RecordingStartGateState: String {
    case available
    case busy
    case unsafe
}

final class RecordingStartGateLease {
    let state: RecordingStartGateState
    private var fd: Int32

    init(state: RecordingStartGateState, fd: Int32 = -1) {
        self.state = state
        self.fd = fd
    }

    func release() {
        if fd >= 0 {
            Darwin.close(fd)
            fd = -1
        }
    }

    deinit { release() }
}

private enum ExistingDirectoryResult {
    case opened(Int32)
    case missing
    case unsafe
}

private func openExistingAnchoredDirectory(_ directory: URL) -> ExistingDirectoryResult {
    let components = directory.pathComponents
    guard directory.path.hasPrefix("/"),
          !components.contains("."),
          !components.contains("..") else {
        return .unsafe
    }
    var directoryFD = Darwin.open("/", O_RDONLY | O_DIRECTORY | O_NOFOLLOW)
    guard directoryFD >= 0 else { return .unsafe }
    for component in components where component != "/" {
        let nextFD = component.withCString {
            Darwin.openat(directoryFD, $0, O_RDONLY | O_DIRECTORY | O_NOFOLLOW)
        }
        if nextFD < 0 {
            let failure = errno
            Darwin.close(directoryFD)
            return failure == ENOENT ? .missing : .unsafe
        }
        Darwin.close(directoryFD)
        directoryFD = nextFD
    }
    return .opened(directoryFD)
}

func acquireRecordingStartGate(cacheRoot: URL = IPC_DIR) -> RecordingStartGateLease {
    let lockDirectory = cacheRoot.appendingPathComponent(
        "application-migration",
        isDirectory: true
    )
    let directoryFD: Int32
    switch openExistingAnchoredDirectory(lockDirectory) {
    case .missing:
        return RecordingStartGateLease(state: .available)
    case .unsafe:
        return RecordingStartGateLease(state: .unsafe)
    case .opened(let opened):
        directoryFD = opened
    }
    defer { Darwin.close(directoryFD) }

    var directoryMetadata = stat()
    guard Darwin.fstat(directoryFD, &directoryMetadata) == 0,
          (directoryMetadata.st_mode & S_IFMT) == S_IFDIR,
          directoryMetadata.st_uid == geteuid(),
          (directoryMetadata.st_mode & mode_t(0o777)) == mode_t(0o700) else {
        return RecordingStartGateLease(state: .unsafe)
    }

    let lockFD = "attempt.lock".withCString {
        Darwin.openat(directoryFD, $0, O_RDWR | O_NOFOLLOW)
    }
    if lockFD < 0 {
        return RecordingStartGateLease(state: errno == ENOENT ? .available : .unsafe)
    }
    var lockMetadata = stat()
    guard Darwin.fstat(lockFD, &lockMetadata) == 0,
          (lockMetadata.st_mode & S_IFMT) == S_IFREG,
          lockMetadata.st_uid == geteuid(),
          (lockMetadata.st_mode & mode_t(0o777)) == mode_t(0o600),
          lockMetadata.st_nlink == 1 else {
        Darwin.close(lockFD)
        return RecordingStartGateLease(state: .unsafe)
    }
    if flock(lockFD, LOCK_EX | LOCK_NB) != 0 {
        let failure = errno
        Darwin.close(lockFD)
        return RecordingStartGateLease(
            state: failure == EWOULDBLOCK || failure == EAGAIN ? .busy : .unsafe
        )
    }
    return RecordingStartGateLease(state: .available, fd: lockFD)
}

class SocketServer {
    let recorder: AudioRecorder
    var sock: Int32 = -1
    private(set) var ownsSocketPath = false
    private var boundSocketIdentity: (dev_t, ino_t)?
    private var quitting = false
    private let startupLock = NSLock()
    private var startupComplete = false
    // Publishing a socket is not a ready recording: recovery updates state and bridge first.
    func finishStartup() { startupLock.lock(); startupComplete = true; startupLock.unlock() }
    private func isStartupComplete() -> Bool {
        startupLock.lock(); defer { startupLock.unlock() }; return startupComplete
    }
    private var singletonLockFD: Int32 = -1
    /// Hooks the daemon uses to start/stop ScreenCaptureKit + microphone capture only
    /// while a recording is in flight, so the macOS menu-bar recording indicator
    /// reflects reality. AppDelegate wires these to the CaptureBackend / MicCapture.
    var onRecordingStart: (() -> Void)?
    var onRecordingStop: (() -> Void)?
    /// Synchronously (re-)arm ONLY the system-audio backend, refreshing SYS_READY /
    /// SYS_ERROR before the "start" readiness gate is evaluated. Needed because a
    /// prior sys-disabled (voicemail / mic-only) recording leaves SYS_READY == false;
    /// without re-arming, the gate would read that stale flag and bail with
    /// sys_capture_not_ready even though the tap could be brought up cleanly.
    /// AppDelegate wires this to CaptureBackend.startCapture(). Mic + the (now
    /// idempotent) sys start still run in onRecordingStart after the reply.
    var rearmSysCapture: (() -> Void)?
    var configureMicDevice: ((String?) -> Void)?
    var listAudioDevices: (() -> [String: Any])?

    // Read requests off the accept thread, then route control actions through a
    // serial queue so recorder mutations stay one-at-a-time. Window scans are
    // intentionally isolated below because macOS Accessibility calls can block
    // for long periods and must not starve status/stop requests during a
    // recording.
    private let requestQueue = DispatchQueue(label: "omm.audio-daemon.ipc.read", attributes: .concurrent)
    private let ipcQueue = DispatchQueue(label: "omm.audio-daemon.ipc.control")
    private let windowScanQueue = DispatchQueue(label: "omm.audio-daemon.window-scan")
    private let windowScanLock = NSLock()
    private var windowScanInFlight = false

    // Cap individual request payloads so a runaway/malicious client cannot
    // grow the read buffer without bound. Real requests are <200 bytes.
    private let maxRequestBytes = 64 * 1024

    init(_ r: AudioRecorder) { recorder = r }

    func stop() {
        if sock >= 0 { close(sock); sock = -1 }
        if ownsSocketPath {
            var current = stat()
            if let (device, inode) = boundSocketIdentity,
               lstat(SOCKET_PATH.path, &current) == 0,
               current.st_dev == device, current.st_ino == inode,
               current.st_mode & S_IFMT == S_IFSOCK {
                _ = unlink(SOCKET_PATH.path)
            }
            boundSocketIdentity = nil
            ownsSocketPath = false
        }
        if singletonLockFD >= 0 {
            close(singletonLockFD)
            singletonLockFD = -1
        }
    }

    func start() -> Bool {
        guard let lockFD = acquireExclusiveFileLock(at: SOCKET_LOCK_PATH) else {
            log("Socket: another audio daemon holds the process lock")
            return false
        }
        singletonLockFD = lockFD
        if unixSocketIsReachable(at: SOCKET_PATH) {
            log("Socket: another audio daemon is already active")
            stop()
            return false
        }
        try? FileManager.default.removeItem(at: SOCKET_PATH)
        guard SOCKET_PATH.path.utf8.count < 104 else {
            log("Socket path too long"); stop(); return false
        }
        sock = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard sock >= 0 else { log("Socket: create failed"); stop(); return false }
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        _ = SOCKET_PATH.path.withCString { strncpy(&addr.sun_path.0, $0, min(SOCKET_PATH.path.utf8.count, 103)) }
        let ok = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.bind(sock, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        guard ok == 0 else { log("Socket: bind failed errno=\(errno)"); stop(); return false }
        var bound = stat()
        guard lstat(SOCKET_PATH.path, &bound) == 0 else { stop(); return false }
        boundSocketIdentity = (bound.st_dev, bound.st_ino)
        ownsSocketPath = true
        // status_agent polls at 1Hz, voicemail.cli polls during recording, and
        // shell scripts ping ad-hoc — a backlog of 5 is trivial to overrun if
        // any handler stalls. 64 absorbs realistic bursts without queueing.
        guard Darwin.listen(sock, 64) == 0 else {
            log("Socket: listen failed errno=\(errno)")
            stop()
            return false
        }
        chmod(SOCKET_PATH.path, 0o600)
        log("Socket ready")
        DispatchQueue.global(qos: .background).async { [weak self] in
            guard let self = self else { return }
            while self.sock >= 0 {
                let c = Darwin.accept(self.sock, nil, nil)
                if c >= 0 {
                    self.prepareClient(c)
                    self.requestQueue.async { [weak self] in
                        guard let self = self else { close(c); return }
                        self.dispatchClient(c)
                    }
                } else if errno == EINTR {
                    continue
                } else {
                    log("Socket: accept failed errno=\(errno)")
                    usleep(200_000)
                }
            }
        }
        return true
    }

    /// Per-accepted-fd hardening:
    ///   * SO_RCVTIMEO / SO_SNDTIMEO bound each read/write so a half-dead
    ///     client cannot tie up the IPC queue indefinitely.
    ///   * SO_NOSIGPIPE keeps `write` to a peer that already disconnected
    ///     from raising SIGPIPE and killing the daemon — the documented
    ///     hazard around the `start` action's late hook callback.
    private func prepareClient(_ c: Int32) {
        var tv = timeval(tv_sec: 5, tv_usec: 0)
        _ = setsockopt(c, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        _ = setsockopt(c, SOL_SOCKET, SO_SNDTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        var on: Int32 = 1
        _ = setsockopt(c, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
    }

    private func dispatchClient(_ c: Int32) {
        guard let request = readRequest(c) else {
            close(c)
            return
        }
        if request.action == "windows" {
            handleWindows(c)
            return
        }
        ipcQueue.async { [weak self] in
            defer { close(c) }
            self?.handleParsed(c, action: request.action, json: request.json)
        }
    }

    private func readRequest(_ c: Int32) -> (action: String, json: [String: Any])? {
        // Framing: every in-tree client (Python record_audio /
        // meeting_daemon / voicemail.recorder, Swift status_agent's
        // DaemonClient since #20) writes the request then `shutdown(SHUT_WR)`
        // — read-until-EOF works for everyone. SO_RCVTIMEO (set in
        // prepareClient) bounds the wait at 5s for a misbehaving client.
        //
        // A newline-framing alternative was attempted in an earlier
        // revision of this commit but proved unreliable in practice
        // (`buf.prefix(n).contains(0x0A)` didn't break the loop against
        // the live socket — likely a `&buf` → C `read()` storage-sync
        // issue worth a dedicated debugging pass) and is no longer
        // needed now that the Swift client matches the Python framing.
        var data = Data()
        var buf = [UInt8](repeating: 0, count: 4096)
        while data.count < maxRequestBytes {
            let n = read(c, &buf, buf.count)
            if n <= 0 { break }
            data.append(buf, count: n)
        }
        guard let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let action = json["action"] as? String else { send(c, ["error":"invalid"]); return nil }
        return (action: action, json: json)
    }

    private func handleWindows(_ c: Int32) {
        windowScanLock.lock()
        if windowScanInFlight {
            windowScanLock.unlock()
            send(c, ["windows": [], "busy": true, "warning": "window_scan_busy"])
            close(c)
            return
        }
        windowScanInFlight = true
        windowScanLock.unlock()

        windowScanQueue.async { [weak self] in
            guard let self = self else { close(c); return }
            defer {
                self.finishWindowScan()
                close(c)
            }
            self.send(c, self.scanWindows())
        }
    }

    private func finishWindowScan() {
        windowScanLock.lock()
        windowScanInFlight = false
        windowScanLock.unlock()
    }

    private func handleParsed(_ c: Int32, action: String, json: [String: Any]) {
        var resp: [String: Any]
        guard isStartupComplete() else { send(c, ["error": "capture_initializing"]); return }
        guard !quitting else { send(c, ["error": "capture_shutting_down"]); return }
        switch action {
        case "start":
            guard json["instance_id"] as? String == SERVICE_OWNER,
                  let session = json["session_id"] as? String, UUID(uuidString: session) != nil,
                  let filename = json["filename"] as? String, filename.hasSuffix(".wav"),
                  !filename.contains("/"), !filename.contains("\0"), filename != ".wav" else {
                send(c, ["error": "invalid_capture_request"]); return
            }
            guard !recorder.isRecording else {
                send(c, ["error": "already_recording"]); return
            }
            requestedFilename = filename
            defer { requestedFilename = nil }
            let startGate = acquireRecordingStartGate()
            guard startGate.state == .available else {
                resp = [
                    "error": startGate.state == .busy
                        ? "application_transaction_in_progress"
                        : "application_transaction_lock_unsafe"
                ]
                send(c, resp)
                return
            }
            defer { startGate.release() }
            // Reset SYS_DISABLED per-request so each "start" reflects the caller's intent
            // cleanly (a sys-disabled recording followed by a normal one must NOT inherit
            // the previous flag).
            SYS_DISABLED = (json["sys_disabled"] as? Bool) ?? false
            // Per-request silence threshold: voicemail uses ~3s, meetings use the default.
            // Omitting the field MUST reset to DEFAULT_SILENCE_SEC so a previous short
            // threshold does not leak into the next recording.
            let silenceSeconds: TimeInterval
            if let s = json["silence_seconds"] as? Int, s > 0 {
                silenceSeconds = Double(s)
            } else if let s = json["silence_seconds"] as? Double, s > 0 {
                silenceSeconds = s
            } else {
                silenceSeconds = DEFAULT_SILENCE_SEC
            }
            let silenceThreshold: Float
            if let t = json["silence_threshold"] as? NSNumber {
                let value = t.floatValue
                silenceThreshold = (value >= 0 && value <= 1) ? value : DEFAULT_SILENCE_THRESHOLD
            } else {
                silenceThreshold = DEFAULT_SILENCE_THRESHOLD
            }
            if let uid = json["mic_device"] as? String {
                let trimmed = uid.trimmingCharacters(in: .whitespacesAndNewlines)
                configureMicDevice?(trimmed.isEmpty ? nil : trimmed)
            } else {
                configureMicDevice?(nil)
            }
            // Per-request output directory: voicemails land in ~/yulu/voicemails,
            // meetings use the default RECORDING_DIR. Omitting the field resets
            // to RECORDING_DIR so a previous voicemail does not leak into the
            // next meeting recording.
            let outputDir: URL
            if let dir = json["output_dir"] as? String, !dir.isEmpty {
                guard let safeOutputDir = safeRecordingSubdirectory(dir) else {
                    resp = ["error":"unsafe_output_dir"]
                    send(c, resp)
                    return
                }
                outputDir = safeOutputDir
            } else {
                outputDir = RECORDING_DIR
            }
            recorder.configure(silenceSeconds: silenceSeconds, silenceThreshold: silenceThreshold, outputDir: outputDir,
                               segmentMs: json["segment_ms"] as? Int ?? 1800000)
            // Start the OS capture sources before creating the WAV/replying. The
            // previous deferred-start design returned "recording" while ProcessTap
            // and AVAudioEngine were still starting, so the first seconds after
            // a user pressed record could be missing from the file.
            onRecordingStart?()
            if !SYS_READY && !SYS_DISABLED {
                let sourceError = CaptureInputFormatFault.startError(SYS_ERROR)
                onRecordingStop?()
                resp = ["error":sourceError, "sysReady": SYS_READY, "sysError": SYS_ERROR, "micReady": MIC_READY, "micError": MIC_ERROR]
            } else if !MIC_READY {
                onRecordingStop?()
                resp = ["error":"mic_capture_not_ready", "sysReady": SYS_READY, "sysError": SYS_ERROR, "micReady": MIC_READY, "micError": MIC_ERROR]
            } else if let p = recorder.start(title: json["title"] as? String ?? "meeting") {
                guard saveBridge(["session_id": session, "files": [p], "recording": true,
                                  "origin_instance_id": SERVICE_OWNER, "started_wall_ms": Int(Date().timeIntervalSince1970 * 1000), "current_path": p, "current_sequence": 0, "current_offset_frames": 0,
                                  "segment_ms": min(1800000, max(60000, json["segment_ms"] as? Int ?? 1800000))]) else {
                    _ = recorder.stop(); onRecordingStop?(); bridgeError = "capture_recovery_state_failed"
                    send(c, ["error": "capture_recovery_state_failed"]); return
                }
                if CommandLine.arguments.contains("--ipc-fixture") {
                    recorder.onSysAudio([Int16](repeating: 512, count: 96000))
                    recorder.onMicAudio([Float](repeating: 0.125, count: 48000))
                    _ = recorder.currentFilePath // Fixture acknowledgement follows its queued synthetic PCM.
                }
                resp = ["status":"recording", "file":p, "session_id": session, "instance_id": SERVICE_OWNER]
            }
            else {
                onRecordingStop?()
                resp = ["error":"start_failed"]
            }
        case "stop":
            guard json["instance_id"] as? String == SERVICE_OWNER,
                  json["session_id"] as? String == readBridge()["session_id"] as? String else {
                send(c, ["error": "capture_session_conflict"]); return
            }
            let wasRecording = recorder.isRecording
            let (p, d) = recorder.stop()
            let sourceStopError = CaptureInputFormatFault.stopError(SYS_ERROR, clock: recorder.captureClock)
            // Only tear down capture if we actually started it. A spurious "stop"
            // (e.g. client retry after a start_failed) would otherwise log a fake
            // "Sys capture idle" / "Mic idle" while neither was running.
            if wasRecording { onRecordingStop?() }
            guard bridgeError.isEmpty else { send(c, ["error": "capture_recovery_state_failed"]); return }
            if let fault = sourceStopError { send(c, ["error": fault]); return }
            var bridge = readBridge(); bridge["recording"] = false
            guard saveBridge(bridge) else { send(c, ["error": "capture_recovery_state_failed"]); return }
            resp = ["status":"stopped", "file": p ?? "", "duration": d,
                    "paged_segments": true, "session_id": bridge["session_id"] ?? "",
                    "instance_id": SERVICE_OWNER]
        case "status":
            resp = ["recording": recorder.isRecording, "file": recorder.currentFilePath, "micLevel": recorder.micLevel, "capture_clock": recorder.captureClock, "sysReady": SYS_READY, "sysError": SYS_ERROR, "micReady": MIC_READY, "micError": MIC_ERROR, "meetingMicState": recorder.meetingMicState.rawValue, "serviceOwner": SERVICE_OWNER, "pid": ProcessInfo.processInfo.processIdentifier, "productVersion": PRODUCT_VERSION, "bundleVersion": BUNDLE_VERSION, "captureIPCVersion": CAPTURE_IPC_VERSION, "bridgeError": bridgeError, "session_id": readBridge()["session_id"] ?? "", "instance_id": SERVICE_OWNER, "paged_segments": true]
        case "sealed_segments":
            guard json["instance_id"] as? String == SERVICE_OWNER else { send(c, ["error": "capture_instance_conflict"]); return }
            _ = recorder.currentFilePath
            guard bridgeError.isEmpty else { send(c, ["error": "capture_recovery_state_failed"]); return }
            let bridge = readBridge()
            let after = json["after_sequence"] as? Int ?? -1
            let limit = json["limit"] as? Int ?? 8
            guard after >= -1, limit > 0, limit <= 8,
                  json["session_id"] as? String == bridge["session_id"] as? String else { send(c, ["error": "capture_session_conflict"]); return }
            var page = [[String: Any]](); var more = false
            let files = bridge["files"] as? [String] ?? []
            guard after < max(1, files.count) else { send(c, ["error": "capture_cursor_conflict"]); return }
            let first = after + 1
            let upper = min(files.count, first + limit + 1)
            for index in first..<upper {
                let path = files[index]
                let url = URL(fileURLWithPath: path)
                guard let anchor = AnchoredRecordingDirectory(root: RECORDING_DIR, target: url.deletingLastPathComponent()) else { send(c, ["error": "capture_recovery_state_failed"]); return }
                let name = url.lastPathComponent + ".sealed.json"
                guard let descriptor = anchor.openExistingFile(named: name) else {
                    if index == files.count - 1 && (bridge["recording"] as? Bool == true) { break }
                    if index == files.count - 1, let audio = anchor.openExistingFile(named: url.lastPathComponent) {
                        defer { try? audio.close() }
                        if (try? audio.seekToEnd()) == 82 {
                            try? audio.seek(toOffset: 0)
                            if let data = try? audio.read(upToCount: 82), data.count == 82 {
                                let h = [UInt8](data)
                                if Array(h[0..<4]) == Array("RIFF".utf8), Array(h[8..<12]) == Array("WAVE".utf8),
                                   Array(h[12..<16]) == Array("fmt ".utf8), h[20] == 1, h[22] == 2,
                                   Array(h[24..<28]) == [0x80, 0xbb, 0, 0], h[34] == 16,
                                   Array(h[74..<78]) == Array("data".utf8), h[78..<82].allSatisfy({ $0 == 0 }) { break }
                            }
                        }
                    }
                    send(c, ["error": "capture_recovery_state_failed"]); return
                }
                defer { try? descriptor.close() }
                guard let bytes = try? descriptor.read(upToCount: 4097), bytes.count <= 4096,
                      let doc = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any],
                      doc["sequence"] as? Int == index, doc["path"] as? String == path,
                      doc["session_id"] as? String == bridge["session_id"] as? String,
                      doc["instance_id"] as? String == bridge["origin_instance_id"] as? String else { send(c, ["error": "capture_recovery_state_failed"]); return }
                if page.count == limit { more = true; break }
                let descriptorHash = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
                let receiptName = name + ".hash.json"
                var hash = ""
                if let receipt = anchor.openExistingFile(named: receiptName) {
                    defer { try? receipt.close() }
                    guard let data = try? receipt.read(upToCount: 1025), data.count <= 1024,
                          let saved = try? JSONSerialization.jsonObject(with: data) as? [String: String],
                          saved["descriptor_sha256"] == descriptorHash, let savedHash = saved["sha256"] else { send(c, ["error": "capture_recovery_state_failed"]); return }
                    hash = savedHash
                } else {
                    // Socket work, not the PCM queue: bound memory and persist one immutable source hash.
                    guard let audio = anchor.openExistingFile(named: url.lastPathComponent) else { send(c, ["error": "capture_recovery_state_failed"]); return }
                    defer { try? audio.close() }
                    var hasher = SHA256()
                    do { while let block = try audio.read(upToCount: 65536), !block.isEmpty { hasher.update(data: block) } }
                    catch { send(c, ["error": "capture_recovery_state_failed"]); return }
                    hash = hasher.finalize().map { String(format: "%02x", $0) }.joined()
                    guard let data = try? JSONSerialization.data(withJSONObject: ["sha256": hash, "descriptor_sha256": descriptorHash]),
                          anchor.publishDescriptor(named: receiptName, data: data) else { send(c, ["error": "capture_recovery_state_failed"]); return }
                }
                page.append(["document": doc, "sha256": hash, "descriptor_sha256": descriptorHash])
            }
            resp = ["instance_id": SERVICE_OWNER, "session_id": bridge["session_id"] ?? "", "origin_instance_id": bridge["origin_instance_id"] ?? "", "segments": page, "has_more": more]
        case "audio_devices":
            resp = listAudioDevices?() ?? ["error": "coreaudio_device_provider_unavailable"]
        case "quit":
            guard json["instance_id"] as? String == SERVICE_OWNER else {
                send(c, ["error": "capture_instance_conflict"]); return
            }
            guard json["captureIPCVersion"] as? Int == CAPTURE_IPC_VERSION else {
                send(c, ["error": "capture_protocol_conflict"]); return
            }
            guard !recorder.isRecording else { send(c, ["error": "already_recording"]); return }
            quitting = true
            send(c, ["status": "bye", "instance_id": SERVICE_OWNER,
                     "captureIPCVersion": CAPTURE_IPC_VERSION, "pid": ProcessInfo.processInfo.processIdentifier])
            // Fixture uses dispatchMain, not an NSApplication event loop. Both
            // paths clean only this listener's inode; ack alone is not exit proof.
            if CommandLine.arguments.contains("--ipc-fixture") {
                stop()
                exit(0)
            }
            DispatchQueue.main.async { NSApp.terminate(nil) }
            return
        default: resp = ["error":"unknown: \(action)"]
        }
        send(c, resp)
    }

    private func send(_ c: Int32, _ d: [String: Any]) {
        guard let data = try? JSONSerialization.data(withJSONObject: d) else { return }
        data.withUnsafeBytes { if let b = $0.baseAddress { _ = write(c, b, data.count) } }
    }

    func scanWindows() -> [String: Any] {
        if CommandLine.arguments.contains("--ipc-fixture") {
            return ["windows": [["app": "Fixture", "title": "Synthetic meeting"]]]
        }
        let workspace = NSWorkspace.shared
        var results: [[String: String]] = []
        let apps = workspace.runningApplications
        for app in apps where app.activationPolicy == .regular {
            let appElem = AXUIElementCreateApplication(app.processIdentifier)
            var winList: CFTypeRef?
            guard AXUIElementCopyAttributeValue(appElem, kAXWindowsAttribute as CFString, &winList) == .success,
                  let wins = winList as? [AXUIElement] else { continue }
            for w in wins {
                var title: CFTypeRef?
                AXUIElementCopyAttributeValue(w, kAXTitleAttribute as CFString, &title)
                if let t = title as? String, !t.isEmpty {
                    results.append(["app": app.localizedName ?? "", "title": t])
                }
            }
        }
        return ["windows": results]
    }
}

func resumeInterruptedRecording(_ rec: AudioRecorder, _ ss: SocketServer) {
    if let title = interruptedRecordingTitle(),
           let previous = readStateDict()?["audio_path"] as? String,
           let directory = safeRecordingSubdirectory(URL(fileURLWithPath: previous).deletingLastPathComponent().path),
           let session = readBridge()["session_id"] as? String, UUID(uuidString: session) != nil {
            rec.configure(silenceSeconds: DEFAULT_SILENCE_SEC, silenceThreshold: DEFAULT_SILENCE_THRESHOLD, outputDir: directory,
                          segmentMs: readBridge()["segment_ms"] as? Int ?? 1800000)
            let startGate = acquireRecordingStartGate()
            defer { startGate.release() }
            guard startGate.state == .available else {
                log("Interrupted recording resume deferred by application transaction"); return
            }
            guard let anchor = AnchoredRecordingDirectory(root: RECORDING_DIR, target: directory) else { return }
            let priorState = readStateDict() ?? [:]
            let legacyState = priorState["sequence"] == nil && priorState["offset_frames"] == nil && priorState["segment_ms"] == nil
            var initialBridge = readBridge()
            if initialBridge["origin_instance_id"] == nil {
                guard legacyState, initialBridge["files"] as? [String] == [previous] else { bridgeError = "capture_recovery_state_failed"; return }
                initialBridge["origin_instance_id"] = SERVICE_OWNER
                if let started = priorState["started_at"] as? String, let date = ISO8601DateFormatter().date(from: started) {
                    initialBridge["started_wall_ms"] = Int(date.timeIntervalSince1970 * 1000)
                }
                guard saveBridge(initialBridge) else { return }
            }
            var sequence = 0; var offset = 0
            let recoveryFiles = readBridge()["files"] as? [String] ?? []
            var logicalFiles: [String] = []
            for path in recoveryFiles {
                let url = URL(fileURLWithPath: path)
                guard url.deletingLastPathComponent() == directory else { bridgeError = "capture_recovery_state_failed"; return }
                let name = url.lastPathComponent + ".sealed.json"
                if let handle = anchor.openExistingFile(named: name) {
                    defer { try? handle.close() }
                    guard let bytes = try? handle.read(upToCount: 4097), bytes.count <= 4096,
                          let document = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any],
                          document["schema_version"] as? Int == 1, document["sequence"] as? Int == sequence,
                          document["offset_frames"] as? Int == offset, document["sample_rate"] as? Int == 48000,
                          document["path"] as? String == path, document["session_id"] as? String == session,
                          document["instance_id"] as? String == readBridge()["origin_instance_id"] as? String,
                          let frames = document["frame_count"] as? Int,
                          frames > 0, (frames <= 1800 * 48000 || (document["legacy_wav"] as? Bool == true && sequence == 0 && offset == 0 && document["temporary"] as? Bool == false)) else { bridgeError = "capture_recovery_state_failed"; return }
                    logicalFiles.append(path)
                    sequence += 1; offset += frames
                } else {
                    let bridge = readBridge()
                    let currentBound = bridge["current_path"] as? String == path && bridge["current_sequence"] as? Int == sequence && bridge["current_offset_frames"] as? Int == offset
                    // Only an exact, valid empty header is an empty tail. Truncated payloads
                    // cannot be dropped from the logical manifest merely by rounding to zero.
                    guard let input = anchor.openExistingFile(named: url.lastPathComponent) else { bridgeError = "capture_recovery_state_failed"; return }
                    let size = try? input.seekToEnd()
                    if let size = size, size < 86 {
                        try? input.seek(toOffset: 78)
                        guard size == 82, let length = try? input.read(upToCount: 4), length == Data([0,0,0,0]) else {
                            try? input.close(); bridgeError = "capture_recovery_state_failed"; return
                        }
                    }
                    try? input.close()
                    guard (path == previous || currentBound), path == (bridge["files"] as? [String])?.last,
                          let frames = anchor.sealInterruptedWav(named: url.lastPathComponent, legacy: legacyState && sequence == 0 && offset == 0) else {
                        bridgeError = "capture_recovery_state_failed"; return
                    }
                    if frames > 0 {
                        // Keep interrupted originals: they may predate rolling capture and must not become disposable.
                        let oldState = readStateDict() ?? [:]
                        let legacyWav = sequence == 0 && offset == 0 && oldState["sequence"] == nil && oldState["offset_frames"] == nil && oldState["segment_ms"] == nil
                        let document: [String: Any] = ["schema_version":1,"temporary":false,"legacy_wav":legacyWav,"session_id":session,"instance_id":readBridge()["origin_instance_id"] ?? SERVICE_OWNER,"sequence":sequence,"offset_frames":offset,
                            "frame_count":frames,"sample_rate":48000,"path":path]
                        guard let bytes = try? JSONSerialization.data(withJSONObject: document),
                              anchor.publishDescriptor(named: name, data: bytes) else { bridgeError = "capture_recovery_state_failed"; return }
                        logicalFiles.append(path)
                        sequence += 1; offset += frames
                    }
                }
            }
            // Empty originals stay on disk, but never consume a sealed segment sequence.
            var recoveredBridge = readBridge()
            recoveredBridge["files"] = logicalFiles
            recoveredBridge["current_sequence"] = sequence
            recoveredBridge["current_offset_frames"] = offset
            guard saveBridge(recoveredBridge) else { return }
            let originalFilename = requestedFilename
            requestedFilename = "resume-\(UUID().uuidString).wav"
            defer { requestedFilename = originalFilename }
            if let p = rec.start(title: title, sequence: sequence, offsetFrames: offset) {
                appendBridgeFile(p, sequence: sequence, offsetFrames: offset, segmentMs: readBridge()["segment_ms"] as? Int ?? 1800000)
                log("⚠️ Resuming interrupted recording")
                ss.onRecordingStart?()
            }

        }
}

// ─── App Delegate ──────────────────────────────────────

class AppDelegate: NSObject, NSApplicationDelegate {
    var recorder: AudioRecorder?
    var micCapture: MicCapture?
    var audioCapture: CaptureBackend?   // D-02 seam: SCK arm now, tap arm (02-04) drops in here
    var meetingMuteMonitor: MeetingMuteMonitor?
    var socketServer: SocketServer?

    func applicationDidFinishLaunching(_ n: Notification) {
        for directory in [DURABLE_DATA_DIR, IPC_DIR, LOGS_DIR, RECORDING_DIR] {
            try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        }
        if !FileManager.default.fileExists(atPath: LOG_PATH.path) {
            FileManager.default.createFile(atPath: LOG_PATH.path, contents: nil)
        }
        logFile = try? FileHandle(forWritingTo: LOG_PATH)
        _ = try? logFile?.seekToEnd()
        log("🎧 Audio Daemon (pid=\(ProcessInfo.processInfo.processIdentifier))")

        let rec = AudioRecorder(); recorder = rec
        let muteMonitor = MeetingMuteMonitor(recorder: rec); meetingMuteMonitor = muteMonitor
        rec.onStopRequest = { [weak self] in
            guard let self = self else { return }
            if !SYS_DISABLED && launchMeetingSilencePrompt() {
                return
            }
            let wasRecording = self.recorder?.isRecording ?? false
            if wasRecording { logEvent(.stopSilence) }
            _ = self.recorder?.stop()
            var bridge = readBridge(); bridge["recording"] = false; saveBridge(bridge)
            if wasRecording {
                self.meetingMuteMonitor?.stop()
                self.audioCapture?.stopCapture()
                self.micCapture?.stop()
            }
        }

        // Probe TCC permissions on launch (each probe opens its underlying capture
        // briefly, then tears it down) so SYS_READY / MIC_READY are accurate without
        // leaving the macOS menu-bar "recording" indicator on while the daemon is idle.
        if #available(macOS 12.3, *) {
            // ── Capture-backend arm selection (PLAT-02 / D-01 / D-03) ──────
            // On macOS 14.4+ use the Core Audio process tap (ProcessTapBackend):
            // it requests the narrower "System Audio Recording Only" TCC scope,
            // so 14.4+ users escape the recurring ScreenCaptureKit re-permission
            // nag (success criterion 3). On 13–14.3 fall back to the
            // ScreenCaptureKit arm. The macOS FLOOR stays 13+ — 14.4 gates ONLY
            // the new tap arm, it does NOT raise the minimum OS (D-01). Do NOT
            // lower this gate to 14.2: the tap symbols exist at 14.2 but the
            // runtime is only reliable at 14.4 (02-RESEARCH.md:459-462).
            let ac: CaptureBackend
            if #available(macOS 14.4, *) {
                ac = ProcessTapBackend(recorder: rec)
            } else {
                ac = ScreenCaptureKitBackend(recorder: rec)
            }
            audioCapture = ac
            // OMM: no implicit permission probe; explicit start arms the unchanged backends.
        }
        let mic = MicCapture(recorder: rec); micCapture = mic
        // OMM: microphone permission is requested only by an explicit recording start/recovery.

        let ss = SocketServer(rec); socketServer = ss
        // Wire the socket "start"/"stop" actions to the actual capture lifecycle.
        // The daemon is idle (no SCStream, no mic engine) until a recording begins.
        ss.onRecordingStart = { [weak self] in
            self?.audioCapture?.startCapture()
            self?.micCapture?.start()
            if !SYS_DISABLED { self?.meetingMuteMonitor?.start() }
        }
        // Pre-gate sys re-arm (self-heal): refresh SYS_READY/SYS_ERROR with a real
        // arm attempt before the "start" readiness gate, so a stale false left by a
        // prior voicemail can't suppress a meeting recording. Idempotent with the
        // sys start in onRecordingStart above.
        ss.rearmSysCapture = { [weak self] in
            self?.audioCapture?.startCapture()
        }
        ss.configureMicDevice = { [weak self] uid in
            self?.micCapture?.selectedDeviceUID = uid
        }
        ss.listAudioDevices = { MicCapture.availableDevices() }
        ss.onRecordingStop = { [weak self] in
            self?.meetingMuteMonitor?.stop()
            self?.audioCapture?.stopCapture()
            self?.micCapture?.stop()
        }
        guard ss.start() else {
            log("Another audio daemon owns the socket; exiting")
            NSApp.terminate(nil)
            return
        }
        try? "\(ProcessInfo.processInfo.processIdentifier)".write(to: PID_PATH, atomically: true, encoding: .utf8)
        resumeInterruptedRecording(rec, ss)
        ss.finishStartup()
        log("Ready")
    }

    func applicationWillTerminate(_ n: Notification) {
        let ownedSocket = socketServer?.ownsSocketPath ?? false
        recorder?.stop(); socketServer?.stop(); meetingMuteMonitor?.stop(); audioCapture?.stopCapture(); micCapture?.stop()
        if ownedSocket { try? FileManager.default.removeItem(at: PID_PATH) }
        try? logFile?.close()
    }
}

// ─── 入口 ──────────────────────────────────────────────

if CommandLine.arguments.contains("--capture-events-self-test") {
    let directory = RECORDING_DIR.appendingPathComponent("capture-events")
    try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let sink = directory.appendingPathComponent("events.log")
    FileManager.default.createFile(atPath: sink.path, contents: nil)
    logFile = try! FileHandle(forWritingTo: sink)
    for event in CaptureOperationalEvent.allCases { logEvent(event, code: -50) }
    log("private-title/path/device-uid/error-must-not-survive")
    let recorder = AudioRecorder()
    recorder.configure(silenceSeconds: 300, silenceThreshold: 0.001, outputDir: directory)
    assert(saveBridge(["session_id": UUID().uuidString.lowercased(), "origin_instance_id": SERVICE_OWNER, "files": [], "recording": true]))
    assert(recorder.start(title: "Synthetic") != nil)
    recorder.updateMeetingMicState(.muted)
    recorder.updateMeetingMicState(.muted)
    for _ in 0..<10 { recorder.onSysAudio([2000, 2000]) }
    _ = recorder.stop() // drains the recorder queue before checking the log
    _ = recorder.stop() // must not create another recording-stop event
    try! logFile!.close(); logFile = nil
    let lines = try! String(contentsOf: sink, encoding: .utf8).split(separator: "\n")
    for event in ["event=mic_gate code=1", "event=system_nonzero_seen", "event=stop_requested"] {
        guard lines.filter({ $0.hasSuffix(event) }).count == 1 else {
            fputs("capture operational event missing or not transition-bounded\n", stderr); exit(1)
        }
    }
    for event in CaptureOperationalEvent.allCases {
        assert(lines.filter { $0.hasSuffix("event=\(event.rawValue) code=-50") }.count == 1)
    }
    assert(!lines.contains { $0.contains("private-title") || $0.contains("Synthetic") || $0.contains(directory.path) })
    print("capture-events-self-test passed; transition_bounded=true; arbitrary_content_redacted=true")
    exit(0)
}

if CommandLine.arguments.contains("--input-format-lifecycle-self-test") {
    if #available(macOS 14.4, *) {
        ProcessTapBackend(recorder: AudioRecorder()).inputFormatLifecycleSelfTest()
        exit(0)
    }
    exit(1)
}

if CommandLine.arguments.contains("--input-format-self-test") {
    let adapter = CaptureInputRateAdapter()
    var outputFrames = 0
    for _ in 0..<100 {
        let output = adapter.convert([Float](repeating: 0.125, count: 441), rate: 44100, channels: 1)!
        outputFrames += output.count
    }
    assert(abs(outputFrames - 48000) <= 64, "44100 input frames must represent one second at 48000 Hz, got \(outputFrames)")
    assert(adapter.convert([], rate: 44100, channels: 1) == [])
    assert(adapter.convert([0], rate: .nan, channels: 1) == nil)
    assert(adapter.convert([0], rate: 44100, channels: 2) == nil)
    adapter.reset()
    let passthrough: [Float] = [0.125, -0.25, 0.375, -0.5]
    assert(adapter.convert(passthrough, rate: 48000, channels: 2) == passthrough)
    adapter.reset()
    var longFrames = 0
    let chunk = [Float](repeating: 0.125, count: 4410)
    for _ in 0..<18000 { longFrames += adapter.convert(chunk, rate: 44100, channels: 1)!.count }
    // Apple buffers a bounded filter tail; flush it once, never reset per callback.
    assert(abs(longFrames - 1800 * 48000) <= 512, "30-minute streaming tail exceeds 512 frames: \(longFrames)")
    longFrames += adapter.finish()!.count
    assert(abs(longFrames - 1800 * 48000) <= 1, "30-minute finalized clock drift: \(longFrames)")
    assert(adapter.finish() == [])
    adapter.reset()
    let restartedFrames = adapter.convert(chunk, rate: 44100, channels: 1)!.count
    assert(abs(restartedFrames - 4800) <= 512)
    for (micRate, sysRate) in [(16000.0, 48000.0), (44100.0, 48000.0), (48000.0, 44100.0), (48000.0, 48000.0)] {
        let recorder = AudioRecorder()
        let directory = RECORDING_DIR.appendingPathComponent(UUID().uuidString)
        try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        recorder.configure(silenceSeconds: 3600, silenceThreshold: 0, outputDir: directory)
        assert(saveBridge(["session_id": UUID().uuidString.lowercased(), "origin_instance_id": SERVICE_OWNER, "files": [], "recording": true]))
        let path = recorder.start(title: "Synthetic")!
        for _ in 0..<100 {
            recorder.onMicAudio([Float](repeating: 0.125, count: Int(micRate / 100)), sourceRate: micRate)
            recorder.onSysAudio([Int16](repeating: 2000, count: Int(sysRate / 100) * 2), sourceRate: sysRate)
        }
        _ = recorder.stop()
        let wave = try! Data(contentsOf: URL(fileURLWithPath: path))
        let frames = (wave.count - 82) / 4
        assert(abs(frames - 48000) <= 64, "paired stream clock drift: \(frames)")
        func sample(_ index: Int) -> Int16 { Int16(bitPattern: UInt16(wave[82 + index * 2]) | UInt16(wave[83 + index * 2]) << 8) }
        assert(sample(2000) > 0 && abs(Int(sample(2001)) - 2000) <= 2, "mic/system pairing changed")
        let descriptor = try! JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: path + ".sealed.json"))) as! [String: Any]
        let clock = descriptor["capture_clock"] as! [String: Any]
        assert(clock["target_rate_hz"] as! Int == 48000 && clock["format_rejects"] as! Int == 0)
        assert((clock["mic"] as! [String: Any])["rate_hz"] as! Double == micRate)
        assert((clock["system"] as! [String: Any])["rate_hz"] as! Double == sysRate)
        assert((descriptor["frame_count"] as! Int) == frames)
    }
    print("input-format-self-test passed; virtual30min=\(longFrames) frames; streamingLatencyBound=512 target frames; finalizedErrorBound=1 frame"); exit(0)
}

if CommandLine.arguments.count == 3,
   CommandLine.arguments[1] == "--inspect-recording-start-gate" {
    let gate = acquireRecordingStartGate(
        cacheRoot: URL(fileURLWithPath: CommandLine.arguments[2], isDirectory: true)
    )
    let state = gate.state.rawValue
    gate.release()
    let encoded = try! JSONSerialization.data(
        withJSONObject: ["state": state],
        options: [.sortedKeys]
    )
    FileHandle.standardOutput.write(encoded + Data("\n".utf8))
    exit(0)
}

if CommandLine.arguments.contains("--rolling-current-state-self-test") {
    let directory = RECORDING_DIR.appendingPathComponent("rolling-current")
    try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let path = directory.appendingPathComponent("segment.wav").path
    assert(saveBridge(["session_id": UUID().uuidString.lowercased(), "origin_instance_id": SERVICE_OWNER, "files": [path], "recording": true, "segment_ms": 60000]))
    assert(writeState(recording: true, title: "Synthetic", path: path, segmentMs: 60000))
    let limit = 60000 * 48
    let writer = RollingRecordingWriter(directory: AnchoredRecordingDirectory(root: RECORDING_DIR, target: directory)!, filename: "segment.wav", maxFrames: limit)!
    let data = Data(repeating: 12, count: (2 * limit + 48000) * 4)
    writer.append(data)
    let tail = directory.appendingPathComponent("segment-0002.wav")
    let blocked = URL(fileURLWithPath: tail.path + ".sealed.json")
    try! FileManager.default.createDirectory(at: blocked, withIntermediateDirectories: false)
    writer.finalize(); assert(writer.failed)
    let state = readStateDict()!
    assert(state["audio_path"] as? String == tail.path)
    assert(state["sequence"] as? Int == 2 && state["offset_frames"] as? Int == 2 * limit)
    assert(state["segment_ms"] as? Int == 60000)
    try! FileManager.default.removeItem(at: blocked)
    print("rolling-current-state-self-test passed"); exit(0)
}

if CommandLine.arguments.contains("--rolling-frame-self-test") {
    let root = requiredDirectory("OMM_TEST_ROOT")
    let directory = root.appendingPathComponent("rolling-" + UUID().uuidString)
    try! FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let anchor = AnchoredRecordingDirectory(root: root, target: directory)!
    let writer = RollingRecordingWriter(directory: anchor, filename: "segment.wav", maxFrames: 4096)!
    var samples = [Int16]()
    for i in 0..<10000 { samples.append(Int16(i)); samples.append(Int16(-i)) }
    let data = Data(bytes: samples, count: samples.count * 2)
    writer.append(data.subdata(in: 0..<12000))
    writer.append(data.subdata(in: 12000..<data.count))
    writer.finalize()
    assert(!writer.failed)
    var payload = Data()
    var offset = 0
    for (sequence, name) in ["segment.wav", "segment-0001.wav", "segment-0002.wav"].enumerated() {
        let path = directory.appendingPathComponent(name)
        let bytes = try! Data(contentsOf: path)
        payload.append(bytes.dropFirst(82))
        let manifest = try! JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: path.path + ".sealed.json"))) as! [String: Any]
        assert(manifest["sequence"] as! Int == sequence)
        assert(manifest["offset_frames"] as! Int == offset)
        let count = manifest["frame_count"] as! Int
        assert(count > 0 && count <= 4096); offset += count
    }
    assert(offset == 10000 && payload == data)
    // Advance the frame clock to the real 30-minute edge, without recording for 30 minutes.
    let limit = 1800 * 48000
    assert(RollingRecordingWriter.writableFrames(offered: 5, current: limit - 2, limit: limit) == 2)
    assert(RollingRecordingWriter.writableFrames(offered: 3, current: 0, limit: limit) == 3)
    assert(RollingRecordingWriter.writableFrames(offered: 1, current: limit, limit: limit) == 0)
    assert(RollingRecordingWriter(directory: anchor, filename: "bad.wav", maxFrames: 1800 * 48000 + 1) == nil)
    let empty = RollingRecordingWriter(directory: anchor, filename: "empty.wav", maxFrames: 1800 * 48000)!
    empty.finalize(); assert(!FileManager.default.fileExists(atPath: directory.appendingPathComponent("empty.wav").path))
    print("rolling-frame-self-test passed")
    exit(0)
}

if CommandLine.arguments.contains("--path-contract-self-test") {
    let manager = FileManager.default
    let testParent = requiredDirectory("OMM_TEST_ROOT")
    let testRoot = testParent
        .appendingPathComponent("yulu-audio-path-contract-\(UUID().uuidString)", isDirectory: true)
    let approved = testRoot.appendingPathComponent("approved", isDirectory: true)
    let movedApproved = testRoot.appendingPathComponent("moved-approved", isDirectory: true)
    let external = testRoot.appendingPathComponent("external", isDirectory: true)
    try! manager.createDirectory(at: approved, withIntermediateDirectories: true)
    try! manager.createDirectory(at: external, withIntermediateDirectories: true)
    let cachedApprovedPath = canonicalDirectory(approved.path)!
    guard let initialAnchor = AnchoredRecordingDirectory(
        root: cachedApprovedPath,
        target: cachedApprovedPath
    ), let initialWriter = WavWriter(directory: initialAnchor, filename: "initial.wav") else {
        fputs("path-contract self-test could not anchor approved root: \(cachedApprovedPath.path)\n", stderr)
        try? manager.removeItem(at: testRoot)
        exit(1)
    }
    initialWriter.finalize()
    try! manager.moveItem(at: approved, to: movedApproved)
    try! manager.createSymbolicLink(at: approved, withDestinationURL: external)

    if let redirected = AnchoredRecordingDirectory(
        root: cachedApprovedPath,
        target: cachedApprovedPath
    ) {
        _ = WavWriter(directory: redirected, filename: "redirected.wav")
    }
    if manager.fileExists(atPath: external.appendingPathComponent("redirected.wav").path) {
        fputs("root swap unexpectedly created external audio\n", stderr)
        try? manager.removeItem(at: testRoot)
        exit(1)
    }
    try? manager.removeItem(at: testRoot)
    print("path-contract-self-test passed")
    exit(0)
}

if CommandLine.arguments.contains("--self-test") {
    let socketTestRoot = requiredDirectory("OMM_TEST_ROOT")
    let socketTestDirectory = socketTestRoot
        .appendingPathComponent("c-\(UUID().uuidString.prefix(6))")
    try! FileManager.default.createDirectory(at: socketTestDirectory, withIntermediateDirectories: true)
    let socketTestPath = socketTestDirectory.appendingPathComponent("daemon.sock")
    let socketTestFD = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
    assert(socketTestFD >= 0)
    var socketTestAddress = sockaddr_un()
    socketTestAddress.sun_family = sa_family_t(AF_UNIX)
    _ = socketTestPath.path.withCString {
        strncpy(&socketTestAddress.sun_path.0, $0, min(socketTestPath.path.utf8.count, 103))
    }
    let socketTestBind = withUnsafePointer(to: &socketTestAddress) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
            Darwin.bind(socketTestFD, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
        }
    }
    assert(socketTestBind == 0)
    assert(Darwin.listen(socketTestFD, 1) == 0)
    assert(unixSocketIsReachable(at: socketTestPath))
    assert(FileManager.default.fileExists(atPath: socketTestPath.path))
    close(socketTestFD)

    let lockTestPath = socketTestDirectory.appendingPathComponent("daemon.lock")
    let firstLockFD = acquireExclusiveFileLock(at: lockTestPath)
    assert(firstLockFD != nil)
    assert(acquireExclusiveFileLock(at: lockTestPath) == nil)
    close(firstLockFD!)
    let secondLockFD = acquireExclusiveFileLock(at: lockTestPath)
    assert(secondLockFD != nil)
    close(secondLockFD!)
    try? FileManager.default.removeItem(at: socketTestDirectory)

    assert(MeetingMuteClassifier.classifyControlLabel("Unmute My Audio") == .muted)
    assert(MeetingMuteClassifier.classifyControlLabel("开启麦克风 (⌘D)") == .muted)
    assert(MeetingMuteClassifier.classifyControlLabel("Mute") == .unmuted)
    assert(MeetingMuteClassifier.classifyControlLabel("关闭麦克风 (⌘D)") == .unmuted)
    assert(MeetingMuteClassifier.classifyControlLabel("Ask to Unmute") == nil)
    assert(MeetingMuteClassifier.isSupportedMeetingWindow(
        appName: "Google Chrome",
        bundleID: "com.google.Chrome",
        title: "Meet - abc-defg-hij"
    ))
    assert(!MeetingMuteClassifier.isSupportedMeetingWindow(
        appName: "Google Chrome",
        bundleID: "com.google.Chrome",
        title: "Gmail"
    ))
    assert(micSamplesForRecording([0.5], gain: 1, meetingState: .muted) == [0])
    assert(micSamplesForRecording([0.5], gain: 1, meetingState: .unknown)[0] > 0)

    var recovery = ZeroBufferRecoveryPolicy(threshold: 3, maxAttempts: 2)
    assert(!recovery.observe(allZero: true, running: true, rebuilding: false))
    assert(!recovery.observe(allZero: true, running: true, rebuilding: false))
    assert(recovery.observe(allZero: true, running: true, rebuilding: false))
    recovery.commitRecoveryAttempt()
    let firstEpisode = recovery.episode
    assert(!recovery.observe(allZero: true, running: true, rebuilding: false))
    assert(!recovery.observe(allZero: false, running: true, rebuilding: false))
    assert(recovery.episode != firstEpisode)
    assert(!recovery.observe(allZero: true, running: true, rebuilding: false))
    assert(!recovery.observe(allZero: true, running: true, rebuilding: false))
    assert(recovery.observe(allZero: true, running: true, rebuilding: false))
    recovery.commitRecoveryAttempt()
    assert(!recovery.observe(allZero: false, running: true, rebuilding: false))
    assert(!recovery.observe(allZero: true, running: true, rebuilding: false))
    assert(!recovery.observe(allZero: true, running: true, rebuilding: false))
    assert(!recovery.observe(allZero: true, running: true, rebuilding: false))
    recovery.resetForRecording()
    assert(!recovery.observe(allZero: true, running: true, rebuilding: false))
    assert(!recovery.observe(allZero: true, running: true, rebuilding: false))
    assert(recovery.observe(allZero: true, running: true, rebuilding: false))

    let recorder = AudioRecorder()
    recorder._selfTestSetSilenceState(
        recording: true,
        micLast: Date(timeIntervalSinceNow: -2),
        sysLast: Date(timeIntervalSinceNow: -2),
        silenceSeconds: 1
    )
    assert(recorder.silenceExpired())
    recorder._selfTestSetSilenceState(
        recording: true,
        micLast: Date(),
        sysLast: Date(timeIntervalSinceNow: -2),
        silenceSeconds: 1
    )
    assert(!recorder.silenceExpired())
    let epoch = Date(timeIntervalSince1970: 1000)
    recorder._selfTestSetSilenceState(recording: true, micLast: epoch, sysLast: epoch)
    assert(!recorder.silenceExpired(now: epoch.addingTimeInterval(299)))
    assert(recorder.silenceExpired(now: epoch.addingTimeInterval(300)))
    recorder._selfTestSetSilenceState(recording: true, micLast: epoch.addingTimeInterval(1), sysLast: epoch)
    assert(!recorder.silenceExpired(now: epoch.addingTimeInterval(300)))
    print("audio_daemon self-test ok")
    exit(0)
}

if CommandLine.arguments.contains("--callback-self-test") {
    let marker = requiredDirectory("OMM_TEST_ROOT").appendingPathComponent("callback-ready")
    let fd = open(marker.path, O_CREAT | O_EXCL | O_WRONLY | O_NOFOLLOW, 0o600)
    guard fd >= 0 else { exit(2) }; close(fd)
    print("callback fixture reached producer start utc_ms=\(Int(Date().timeIntervalSince1970 * 1000))"); fflush(stdout)
    exit(launchMeetingSilencePrompt() ? 0 : 1)
}
if CommandLine.arguments.contains("--ipc-fixture") {
    umask(0o077)
    let rec = AudioRecorder()
    let server = SocketServer(rec)
    server.onRecordingStart = { SYS_READY = true; MIC_READY = true }
    server.onRecordingStop = { SYS_READY = false; MIC_READY = false }
    server.listAudioDevices = { ["input": [["uid": "fixture-mic", "name": "Synthetic input"]], "output": [], "default_input_uid": "fixture-mic"] }
    guard server.start() else { exit(1) }
    resumeInterruptedRecording(rec, server)
    if rec.isRecording {
        rec.onSysAudio([Int16](repeating: 512, count: 96000))
        rec.onMicAudio([Float](repeating: 0.125, count: 48000))
        _ = rec.currentFilePath // barrier: synthetic payload and current state are ready together
    }
    server.finishStartup()
    print("IPC fixture endpoint ready utc_ms=\(Int(Date().timeIntervalSince1970 * 1000))"); fflush(stdout)
    dispatchMain()
}

umask(0o077)
let app = NSApplication.shared
Darwin.signal(SIGPIPE, SIG_IGN)
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()

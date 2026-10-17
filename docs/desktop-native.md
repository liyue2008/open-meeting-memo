# Desktop native implementation

The desktop host is a macOS Apple Silicon Tauri 2 application. The UI calls only the [versioned desktop interface](../contracts/desktop/v1/README.md); it cannot provide filesystem paths, execute programs, or upload audio. React and TypeScript are owned by the separate frontend lane. This document describes the native component, not final integrated release acceptance.

## Ownership and entry points

- `src/application/`: shared CLI/service/desktop use cases, frozen request/response validation, profile generations, configuration, imports, artifacts, media, bundle verification and explicit service lifecycle.
- `src/core/store/desktop.rs`: SQLite meeting metadata, content search, sorting/pagination, retained trash, artifact/task projections, cancellation and persistent request deduplication. `src/core/store/transaction.rs` keeps nested use cases inside the outer write transaction.
- `apps/desktop/src-tauri/src/host.rs`: trusted local `main` window admission, native dialogs/clipboard, finite command dispatch and identity-bound service requests. `src/lib.rs` declares the individual commands. Tauri build-generated allow permissions are granted only to the local main capability; there is no remote capability or shell/filesystem plugin.
- `packaging/desktop/`: locked arm64 package assembly, dependency and license/source redistribution, immutable resource manifest, metadata-only installation preflight and versioned side-by-side user installation.

The imported Swift/Yulu audio, PCM, synchronization, WAV and recovery code remains untouched. Existing sherpa-onnx C API/model verification, Pi isolation and XDG artifact security remain in their existing layers.

## Persistence and safety choices

- Metadata uses compare-and-swap revisions. Conflicting editors reload instead of overwriting. Active/unsealed work cannot be renamed or put in Trash.
- Delete means **retained Trash**, not filesystem deletion. Restore retains the same meeting ID, audio, artifacts and attempts. This release exposes no permanent-delete operation.
- Library text search includes meeting metadata and verified committed transcript/summary content. Date boundaries are supplied by the UI; filtering, stable sorting and pagination run in the native use case. Cursors carry a result revision and reject changed pages.
- Current artifacts are the newest verified committed results, not the newest failed attempt. A summary is current only when its transcript dependency matches the selected current transcript. All committed versions remain readable with meeting/task/stage cross-checks.
- Reprocessing creates a new generation; summary-only work references an exact verified transcript. Concurrent active work is rejected and request IDs are persistently deduplicated. Automatic retry retains the existing initial attempt plus at most three retries.
- Cancellation first persists intent. The pipeline drops its owned operation, drains its owned worker/Pi children, then settles cancellation. Cancellation and commit serialize through SQLite; a completed commit wins rather than being relabeled cancelled. Existing task enums remain compatible on disk; cancellation is separately persisted and projected as `cancelled` through the desktop contract. Explicit requeue creates a new generation.
- Imports are selected by a native file dialog, opened with bounded no-follow reads, privately staged and normalized using the pinned local codec. Renderer-visible handles are profile/generation-bound, expiring and finite. Only PCM WAV/Ogg Opus input is accepted; size/duration/decoded limits and the original source hash are retained. Imported mixed audio is recorded as mixed lineage rather than invented microphone/system separation. Commit runs through the compatible service and retains an import receipt.
- All configuration writers use the same no-follow writer lock and raw-file SHA-256 compare-and-swap. A per-request journal recovers a publish that completed before its reply was recorded. Legacy one-hour segment configuration has the existing in-memory normalization; observing does not rewrite it. Secrets do not belong in OMM configuration.
- Opening an old profile is read-only and does not migrate SQLite. A versioned capability request uses the existing peer/instance-bound Unix socket. A missing socket alone never authorizes writes: offline configuration writes additionally require an explicitly initialized/taken-over profile generation, the persistent service lock and writer lock, then a generation recheck. Other mutations require the compatible running service.
- Profile initialization and service start/stop/switch are explicit planned operations with short-lived, single-use native plan tokens. Unknown service executables, active recording/work, failed identity checks and changed snapshots block replacement. The app never guesses a process from its PID or silently installs/restarts the personal LaunchAgent. Existing config/authentication/data are not copied or replaced by application installation.

## Media and local text output

`omm-media://localhost/<grant>` is registered by the native host. Grants are finite, short-lived and owned by the trusted main window. There is no general path endpoint. Opening a grant verifies the stored artifact SHA-256 through an `openat`/`O_NOFOLLOW` descriptor chain and holds the audio maintenance reader lock. Reads remain bound to that descriptor. Metadata changes trigger re-verification; changes during a read fail rather than returning unverified bytes.

The protocol supports GET/HEAD, one bounded byte range, suffix ranges, ETag/If-Range and finite responses. Invalid/multiple ranges, stale manifests, oversized responses, symlinks, unsafe ownership/modes and unknown grants are rejected. Closing the grant releases its reader lock. Native exports use the native save dialog and create a new 0600 file without replacing existing content; clipboard text comes only from a verified selected artifact, with a 1 MiB bound. Meeting Markdown is returned as inert text, never executable HTML.

## Reproducible component commands

From the worktree root, with the existing Rust/Xcode and project Node toolchains:

```sh
cargo fmt --all --check
cargo test --locked --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --workspace
cargo test --locked -p omm-desktop --all-targets
node scripts/generate-desktop-contracts.mjs --check
bash packaging/capture/build-dev.sh
OMM_CAPTURE_TEST_BINARY="$PWD/target/capture/OMMCapture.app/Contents/MacOS/omm-capture" cargo test --locked --test capture_adapter -- --ignored
OMM_CAPTURE_TEST_BINARY="$PWD/target/capture/OMMCapture.app/Contents/MacOS/omm-capture" cargo test --locked --test core_integration review_r1_03_ -- --ignored
bash packaging/desktop/test-wkwebview.sh
```

The WKWebView driver compiles the explicit `native-bridge-fixture` feature, uses fresh `/private/tmp` XDG roots, disables notifications and only seeds synthetic text plus silent WAV. It invokes the production command validation, Tauri permissions, service socket, store and media handler in an actual trusted `tauri://localhost` WKWebView. Its result records 41 assertions and `playCalls: 0`, including actual import commit/cancel/path rejection, fake native export/clipboard, configuration conflicts, retained summary generations, cancellation/requeue and identity-bound fake capture start/status/seal. It decodes silence without an audio output node, reads metadata and performs a paused seek; it never calls `play`. Native test evaluation channels are absent from the production feature set. An integration-owned driver can be supplied with `OMM_WKWEBVIEW_DRIVER`; the completion object is `window.__ommFixtureResult`.

The focused regressions are source-named in `tests/desktop_native.rs`: concurrent metadata CAS, transaction rollback/idempotency, retained Trash/restore, search/cursors/version retention, active-work guards, config lock/hash/journal recovery, legacy read-only and future schema protection, import normalization/lineage/generation, persistent cancellation and verified media/ranges/permissions/symlinks/external changes. Existing controlled adapter tests continue covering actual owned-child reaping. No test proves real speech recognition, cloud authentication, meeting detection, desktop notification visibility, hardware permission or long-meeting quality.

## Packaging and installation

```sh
# Requires clean committed source. Does not launch helpers or services.
bash packaging/desktop/package.sh
# Metadata-only preflight; no config/authentication content is read.
bash packaging/desktop/install.sh --check '/absolute/path/Open Meeting Memo.app'
# Explicit versioned side-by-side installation under ~/Applications.
bash packaging/desktop/install.sh '/absolute/path/Open Meeting Memo.app'
```

The package contains its own CLI, ASR worker, identity-preserving capture app, pinned sherpa-onnx/ONNX Runtime libraries, local FFmpeg/Opus toolchain, notices and dependency source archives. Model weights stay external and hash-manifest bound. Pi/Node and Lark remain external integrations; their absence is diagnosed, not hidden by fallback. There is no Python product runtime; the Python files here run only during packaging/installation/tests.

Assembly checks arm64, macOS 15.5 deployment target, dynamic-library closure, capture bundle identity/entitlements, inside-out ad-hoc signatures, bundled licenses/sources and file hashes. The application icon is generated from the original approved OMM glyph geometry. Ad-hoc signing is development signing, not notarization.

Installation verifies the source bundle and copies it to a revision-named new user app bundle. It does not modify `~/.local/bin/omm`, LaunchAgents, personal recording/configuration/authentication, consent or auto-record settings. Its 0600 receipt contains only non-secret package metadata. Integrated release acceptance must rebuild from the final combined clean commit, verify that exact package/install provenance and actually launch the production package with an isolated profile. A component package or fixture is not that final release acceptance.

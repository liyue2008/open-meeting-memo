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

## Main-window close ACL and isolated WebKit fixtures

The only additional core permissions are `core:event:allow-listen`,
`core:event:allow-unlisten` and `core:window:allow-destroy`, scoped to the local
`main` window in the existing capability. Tauri's `onCloseRequested` registers a
window-targeted event listener and destroys that window only after the handler
has not prevented the close; the UI's confirmed discard path also calls
`destroy`. No event emit, IPC close, window creation, core default, remote,
filesystem or shell permission is granted. The 36 business permissions and
native trusted-origin checks are unchanged. Production defines just one window;
the frontend cannot create others with these permissions.

```sh
# Fresh evidence directory required, from this worktree; no shared target/dist.
OMM_WKWEBVIEW_SCENARIO=window-acl \
OMM_WKWEBVIEW_EVIDENCE="$(mktemp -d /private/tmp/omm-window-acl-XXXXXX)" \
bash packaging/desktop/test-wkwebview.sh
```

This API-level real-WK fixture uses the production capability and actual
`tauri://localhost` origin, but an inert fixture HTML document so React's own
close listener cannot compete with the probe. It proves listen, pending-close
preservation, unlisten cleanup and actual current-main destruction. A native-only
unknown local window and a main-labelled, short-lived owned loopback HTTP page
prove the same commands are refused for untrusted callers/origins. Registered
custom protocols are classified as local by Tauri and are not remote negatives.
No production test command or window-creation permission is added. Complete
React dirty-form cancel/save/discard behaviour remains combined UI acceptance.

**Every `native-bridge-fixture` window is nonpersistent/incognito**, including
normal/native and UI-driver runs. This is a feature-only context override;
production `tauri.conf.json` and appearance persistence are unchanged. The locked
Tauri 2.12.4 / tauri-runtime-wry 2.12.2 / Wry 0.57.0 path forwards `incognito`
to `WKWebsiteDataStore::nonPersistentDataStore`. The window-ACL scenario verifies
two same-origin independent windows with a random synthetic localStorage key:
B cannot see A, B writes its value, A still sees A. It never reads personal
WebKit files or clears global stores. XDG/HOME isolation alone is not claimed
to isolate WebKit. This evidence does not prove cross-process preference
persistence; ephemeral fixtures can test same-window reload only.

### Larger UI scene and owned-view screenshots (test feature only)

Set `OMM_WKWEBVIEW_SCENARIO=ui` for 26 private synthetic meetings instead of the
unchanged default one-meeting/one-segment native 41-check scene. The original
`Private bridge fixture` has two 3-second silent WAV segments (6 seconds total).
The added 25 meetings have stable `UI ... fixture` titles, including one failed
and one cancelled task. `ui-scenario.json` in the evidence directory records
meeting/task IDs, titles, statuses and budgets. Use an unfiltered 25-row first
page to reach the second page before other operations; `query="UI "`, title
scope, pageSize=20 selects a stable 25-row subset even while the original
fixture's processing tasks change. Stale revisions must still fail normally.

```sh
OMM_WKWEBVIEW_SCENARIO=ui \
OMM_WKWEBVIEW_DRIVER=/private/tmp/your-owned-driver.js \
OMM_WKWEBVIEW_EVIDENCE="$(mktemp -d /private/tmp/omm-ui-fixture-XXXXXX)" \
bash packaging/desktop/test-wkwebview.sh
```

UI result polling is bounded at 180 seconds (the default remains 35). The
parent-owned process wait is 240 seconds for UI, 90 otherwise; timeout still
terminates/waits only its own child and reports failure. No success is inferred
from a timeout or missing report. Every scene remains nonpersistent/incognito.

A UI driver may request a PNG of **only its current owned main WKWebView** after
its DOM/UI transitions have painted:

```js
window.__ommFixtureSnapshotResult = null;
window.__ommFixtureSnapshotRequest = { name: 'light-expanded' };
// Poll __ommFixtureSnapshotResult, with a finite deadline, for {ok:true,name,bytes}.
// Optional bounded logical window size; only this owned fixture window resizes:
window.__ommFixtureSnapshotRequest = { name: 'dark-narrow', width: 640, height: 700 };
```

Names permit only ASCII letters/digits/hyphens (1–64 characters), with optional
width 360–1600 and height 480–1200 specified together. Unknown keys, paths,
untrusted origins and windows are refused. Files are new 0600 `<name>.png`
children of the private evidence directory; existing files are not overwritten.
Resize waits for the actual window, view bounds and CSS viewport to settle,
preserving the measured titlebar inset rather than guessing its height. PNG
pixels include the view's backing scale. `WKWebView.takeSnapshot` with
`afterScreenUpdates` and in-process NSImage→PNG conversion never captures a
display/other window, requests screen permissions, changes OS appearance or
records video. Drivers change theme/sidebar using this fixture's UI, not system
settings. Optional already-locked objc2/block2 dependencies and this channel
exist only with `native-bridge-fixture`; production has neither test commands nor
additional permissions. These screenshots prove the seam, not final React theme
or dirty-form behaviour; the combined repaired UI still needs that acceptance.

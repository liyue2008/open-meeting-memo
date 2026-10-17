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

`MediaGrant.revision` acknowledges the exact `MediaManifest.revision` supplied as `media_open.expectedRevision`, including when the manifest is paginated. It is not the selected segment's SHA-256. The verified descriptor independently retains the stored artifact hash for verification and HTTP ETag/If-Range; a current manifest revision cannot authorize corrupt audio. Source-named host regressions in `apps/desktop/src-tauri/src/host/media_revision.rs` cover the distinct revisions, incorrect/stale snapshots, independent hash checks and reader-lock release on `media_close`.

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

The ACL probe navigates the same owned `main` view to its bounded loopback HTTP
page for the remote negative, then back to the local origin before actual
permitted destruction. It does not destroy/recreate a label while asynchronous
WK teardown is still settling. Readiness requires both injected IPC and the
exact expected origin. The private HTTP fixture serves only an actual GET;
empty/speculative connections do not consume its one response.

### Owned native close request (feature-only UI scene)

In the `ui` scene only, set `window.__ommFixtureCloseRequest` to
`{id:'cancel-1',action:'close'}`. Poll `window.__ommFixtureCloseResult` for the
matching bounded receipt `{ok:true,id,nativeCloseRequested:true}`. This calls
Rust's actual `Window.close()` on the current local `main`, so React's existing
`onCloseRequested` and dirty-form confirmation decide whether the window stays.
The fixture never grants production close/emit permissions or destroys main.
Unknown windows, nonlocal origins, unknown keys/actions, duplicate IDs and more
than 16 requests are refused. IDs are 1–64 ASCII letters/digits/hyphens.

Before clicking React's actual final discard button, submit
`{id:'finish-1',action:'arm-destroy',result:{ok:true,checks:['...'],playCalls:0}}`
and wait for `{ok:true,armed:true}`. This writes new private
`close-before-destroy.json` with the actual pre-confirmation DOM, then observes
main's native destruction. A hidden, incognito, unprivileged keeper exists only
so the fixture can shut down the owned service, wait the fake capture child and
verify all audio readers were released before writing `bridge-result.json` and
exiting. Missing destruction, timeout or failed cleanup reports failure. WK
callbacks hand native window operations to a worker rather than blocking the WK
event thread. Production has none of this channel.

Direct evidence: `/private/tmp/omm-native-owned-close-mIIkni/close-v2` contains a
real React settings edit → native close → cancel (draft retained, configuration
unchanged) → second native close → actual React discard → native destroy,
`playCalls=0`, acknowledged shutdown, waited capture child and released readers.
The original failed attempts are retained beside it: their driver incorrectly
looked for a settings button instead of the actual navigation link; a temporary
stage diagnostic localized the failure. `close-acl` directly rejects this new
fixture request from an unknown local window and a navigated remote main.
These are component proofs, not a replacement for the final combined UI gate.

### First-use lifecycle proof (feature-only, real bundled CLI)

`OMM_LIFECYCLE_RESOURCE_APP='/absolute/path/to/verified-release.app' bash
packaging/desktop/test-lifecycle.sh` preserves the verified release resources,
including the CLI and original manifest bytes, and builds only the fixture host.
The temporary test app is re-sealed ad hoc (without re-signing nested resources)
and verified; source and staged CLI/manifest hashes must match before and after
execution. Actual running/switched PID executable paths must match that staged
CLI. `provenance.json` separates fixture source from resource source revision;
`cli-byte-preservation.json` records hashes and actual PID receipts. It does not
install or register a LaunchAgent. This temporary app is not the final release.

The `lifecycle` scene requires genuinely absent CONFIG/STATE/DATA/CACHE roots,
a new private runtime, and an explicit 0700 HOME inside the fixture root with no
`Library/LaunchAgents`. The latter follows the CLI's actual registration lookup;
XDG roots alone cannot isolate that lookup. No owner/config/database is seeded.
The real local WK bridge calls `profile_initialize`, `service_plan` and
`service_apply`; actual manifest-bound CLI processes prove start, safe switch
and stop. Auto recording and notifications remain disabled. Only a synthetic
peer executable is configured for the unsealed-recording rejection test.

The test-only local-main channel `__ommLifecycleCase` accepts each of
`configure`, `running`, `switched`, `offline` once, returning
`__ommLifecycleCaseResult`; it exercises persisted busy recording, expired plan,
stale generation and an identity-unknown listener. These auxiliary negatives
call the same Rust lifecycle use cases; they are not claims of extra UI
coverage. The exact native receipts are included under `nativeCases` in the
result, alongside owned CLI PIDs, capture quit/wait, socket/lock cleanup and data
preservation. The driver also verifies byte-equivalent configuration across the
successful lifecycle. Only the current trusted local main is processed.

The scene has a 360-second bound and the owning wrapper 420 seconds: per-call
production resource verification remains intact and cumulative verification
exceeded an earlier 120-second test-only budget. On failure, cleanup may signal
only a still authenticated, exact manifest CLI observed in this private run,
with no persisted recording; forced cleanup always makes the result fail.
There is no production lifecycle/auth/launcher change. Final React first-use
and service-button acceptance remains the integrator's responsibility.

The synthetic summary now has an `Action items` heading above its existing
checkbox-source list item. This makes read-only action extraction nonempty;
older container-only UI evidence is not reclassified as a nonempty-list proof.

#### Owned lifecycle driver override

`OMM_LIFECYCLE_DRIVER=/private/tmp/owned-case/react-lifecycle.js` optionally
replaces the built-in lifecycle driver in `test-lifecycle.sh`. The default is
unchanged. Overrides must be canonical, nonsymlink, singly-linked regular files,
owned by this user with no group/other permissions, valid UTF-8 and smaller than
128 KiB. The wrapper opens without following links, copies the captured bytes
into its fresh private profile and records `driver`/`driverHash` in
`provenance.json`. JavaScript is evaluated as-is (no sample substitution).
The resource verification, CLI launcher, isolated HOME/XDG roots, disabled
notifications/auto recording, native negative receipts and owned cleanup are
unchanged. An actual React driver must still use the documented four native
case checkpoints around its real first-use/start/switch/stop buttons; the
built-in driver is the reference for ordering and final result shape.

```sh
OMM_LIFECYCLE_RESOURCE_APP='/absolute/path/to/current-clean-candidate.app' \
OMM_LIFECYCLE_DRIVER='/private/tmp/owned-case/react-lifecycle.js' \
  bash packaging/desktop/test-lifecycle.sh
```

### Explicit persistent WK test stores (feature-only)

The default fixture remains **nonpersistent**. An explicit `store` scene uses
macOS's public UUID website-store API, available on this deployment target.
It creates two fresh random UUIDs, first checking only their absence in the
identifier registry. Before any local page navigation, the fixture checks the
actual WKWebView's `websiteDataStore.isPersistent` and actual `identifier`
against the requested UUID. It never opens the default store or another
identifier's data. A/B begin empty; distinct localStorage nonce markers prove
that a write in A is absent from B and a write in B cannot replace A's marker.
The storage location is OS-managed, selected by that exact native UUID, not by
HOME/XDG, bundle-name guesses, a custom filesystem path or global cache removal.

```sh
# Requires the already-built frontend distribution; only the locked fixture
# host is incrementally built. Each command owns a fresh private evidence root.
bash packaging/desktop/test-stores.sh proof
bash packaging/desktop/test-stores.sh allocate
# Use ONLY owned-wk-store.json emitted by that allocate invocation.
export OMM_WKWEBVIEW_STORE_MANIFEST=/private/tmp/omm-wk-owned-XXXXXX/owned-wk-store.json
bash packaging/desktop/test-stores.sh write
bash packaging/desktop/test-stores.sh read
bash packaging/desktop/test-stores.sh cleanup
```

`proof` creates A/B, verifies actual identity/isolation, destroys their views,
removes exactly those two stores with the native completion API and verifies
both identifiers are absent. `allocate` performs the same isolation checks but
retains the pair. `write` and `read` are independent App processes proving a
synthetic marker survives process exit. `cleanup` rechecks each owned nonce,
closes its view, removes only the exact identifiers and records `removed:true`
in the private manifest. A removed/foreign/malformed manifest, unknown action,
noncanonical path or concurrent manifest lease is rejected; there is no default
fallback. The manifest is 0600, its parent 0700, and a process-lifetime exclusive
lease prevents simultaneous reuse. Keep a failed run's manifest for this exact
cleanup command; do not clear global WebKit caches. Use the same fixture
executable/application identity for allocation, UI runs and cleanup.

For the real React UI, retain an allocated manifest and run **two separate**
`OMM_WKWEBVIEW_SCENARIO=ui` App processes with that environment and the existing
owned driver interface. Only this explicit combination replaces the default
main with a UUID-identified persistent main. Native identity is verified while
it is still `about:blank`, before navigating to React. Drivers can wait for
`window.__ommOwnedStoreAttached` (`actualPersistent`, `actualIdentifier`,
`ownedMarkerVerified`); the same receipt is `owned-store-attached.json`.
Set theme/sidebar using the actual controls in process one, close it with the
owned close seam, and assert their **initial render** in process two before
changing them. Do not clear localStorage: the ownership marker must survive.
Then run exact `cleanup`. No production permissions, CSP, schema, storage policy
or frontend source changes are involved.

Direct component evidence: `/private/tmp/omm-wk-owned-HCUeOG` contains final
A/B proof (`proof-2.log` → `/private/tmp/omm-wk-owned-0VPHtA`), independent
write/read processes and precise cleanup (`allocate.log`, `write.log`,
`read.log`, `cleanup.log`). `ui-process-1` and `ui-process-2` are different PIDs
using UUID `c4b86d17-e5e1-408a-aee0-4d56d0920b47`: actual React writes dark /
collapsed, exits through dirty-close cancel/confirm, and the next process
restores dark / collapsed before any preference writes. Both report 0 play,
real main destruction, acknowledged shutdown, waited fake capture and released
readers. `ui-cleanup.log` confirms exact A/B removal. All allocated pairs were
removed; a removed-manifest negative fails with exit 1. The lifecycle override
positive also passed with two actual CLI processes and all native negatives;
a nonprivate override was rejected before launching the fixture. These proofs
are not final current integrated/release UI acceptance or personal permission,
ASR, authentication, audible playback or long-meeting validation.

## Held lifecycle negatives (test feature only)

The `lifecycle` scenario additionally accepts a consumed-once JS control slot;
production commands, ACL, CSP, schema, default raw probe and the four existing
native cases are unchanged. Use the existing private driver override, for example:

```sh
OMM_LIFECYCLE_RESOURCE_APP=/absolute/path/to/current-verified.app \
OMM_LIFECYCLE_DRIVER=/private/tmp/owned-case/react-lifecycle.js \
  bash packaging/desktop/test-lifecycle.sh
```

Wait for each native `__ommLifecycleCaseResult` before controlling that phase.
Do not advance `__ommLifecycleCase` while held. From the trusted main only:

```js
window.__ommLifecycleControlResult = null;
window.__ommLifecycleControl = {id: 'ui-negative-1', action: 'hold-busy'};
// Await __ommLifecycleControlResult with this id before observing the real UI.
// {id, action, ok:true, held:'busy', details:{held,expiresAtMs,observedPlan,listenerPid}}
window.__ommLifecycleControl = {id: 'ui-negative-2', action: 'restore'};
// {id, action, ok:true, held:null, details:{restored:'busy',recordingCleared:true,...}}
```

| Action | Required completed native case | Real held state / observation |
| --- | --- | --- |
| `hold-busy` | `running`, verified live manifest-bound CLI | Identity-tagged synthetic recording/unsealed Store state; real stop/switch plans report busy and Core refuses a previously displayed applicable plan. No capture start or hardware call. |
| `hold-unknown` | `configure` or `offline`, no existing runtime socket | Independent owned fixture executable holds RuntimeGuard/socket, not the manifest CLI. Real plans report `unknown`, `canApply=false`, `unowned_service`. Parent pipe and child deadline bound ownership. |
| `stale-plan` | `configure` or `offline`, initialized and offline | First display and retain a real UI plan. The control rotates only this private owner's generation under both normal locks. Apply the old UI plan while held; Core refuses it and the normal Host consumes the token. |
| `restore` | One held state | Clears only the matching synthetic session/directory, or restores exact original owner bytes only if its replacement still matches, or requests listener exit and waits. Then refresh/recheck the actual UI before the next phase. |
| `finish` | `offline` after normal stop and four native cases | Restores any held state and waits the already stopped fake capture peer; seals further controls. Does not terminate the App or fabricate successful cases. Publish the normal `__ommFixtureResult` afterward for owned service/socket/lock final checks. |

`details.observedPlan` is native diagnostic evidence, not a replacement UI plan
or usable Host plan token. Observe disabled/error/recheck behavior through the
real React bridge; never substitute these diagnostics into UI state. The stale
proof checks that the old token is still unexpired, is refused while held, and
remains refused after original generation restoration, while a fresh plan works.
Public errors are sanitized, so a test must not expect private error messages.

Only `id` and `action` are accepted. IDs are unique `[A-Za-z0-9_-]`, 1–64 bytes;
there are at most 32 admitted controls, one held state, a 90-second hold deadline
and the existing 360-second scenario deadline. Unknown fields/actions, duplicate
IDs, wrong phases or holding twice return `{id,action,ok:false,held:null,error,
cleanup}` and restore any held state. Finish before offline is rejected. Timeout
or a normal failure result also restores state, releases/waits the owned
listener, stops the owned CLI/helper and writes a genuine failure exit. Abort
before offline with `__ommFixtureResult={ok:false,playCalls:0,error:'...'}` rather
than trying to manufacture missing cases. Native results include `nativeHeld`
(request receipts, finished, clean) and `nativeHeldCleanup` (final restoration
receipt). No callbacks synchronously create/close windows on the WK event thread.

`apps/desktop/src-tauri/fixtures/lifecycle-held-probe.js` is an optional focused
component probe; the original `lifecycle-probe.js` remains the default. It proves
held/observed/restore/next-phase behavior using the real bridge and Core, not
React disabled rendering. Final real React negatives and the current release
resource provenance remain integration acceptance, using the same control API.

Focused evidence: `/private/tmp/omm-held-lifecycle-hHXQyR`. The original sanitized
error assertion failure and stale duplicate-response driver failure are retained;
both exited normally with restored state and waited processes. The corrected
held probe passes 45 checks/0 play with original verified `fab32d7` resources
(`/private/tmp/omm-lifecycle-proof-LJFXeN`): source/staged/executed CLI SHA-256 is
`bf7fd49b917e23a39a841eacf04ac4d00a128a3a1075dd2f325d53cf8c510061`, original/staged
manifest is `6bc26fe6ceba5b723ea29a36155ba8b375a03133ee3d0185b2fd7ab7e884fe66`.
Actual CLI PIDs 68017/68172 and App 67660 exited; listener receipts confirm graceful
wait and socket/lock release. The unchanged default raw probe also passed with
original older `f139cdd5` resources (`/private/tmp/omm-lifecycle-proof-cT2Uwq`).
A real 90-second held-listener timeout in `expiry/` returns App exit 1 as intended,
records final restoration/wait, and removes the socket/locks; its harness exit 0
means the expected failure was verified, not a successful lifecycle. The focused
Rust tests, feature clippy, nonfixture compile and shell checks pass. Neither old
resource proof is final `b398585b` or later candidate acceptance.

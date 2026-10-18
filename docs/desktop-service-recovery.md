# Desktop service recovery

This recovery is an explicit, two-step operation for a narrowly recognized **stopped legacy profile**. Opening the App does not migrate its database or adopt its service. Existing trusted modern-service switching keeps its original semantics.

## Why a 0.1.0 service can be unknown

The App and an older personal CLI can both report 0.1.0 while containing different code. A version string, UID, executable path or LaunchAgent label is not proof that the App may control that process. Live service control requires the existing bundle manifest/hash verification and compatible capability/instance handshake. An unverified live process receives **no status or shutdown request**.

Missing `desktop-owner.json` means `profile_requires_takeover`, not a proven SQLite schema mismatch. An invalid/future ownership marker or unsupported inspected SQLite schema remains incompatible. Missing/unreadable configuration and missing status fields produce unknown recording/preferences, not idle or a saved automatic-recording preference.

## An old service that is still live or loaded

`external_stop_required` means the App has not stopped the original service. The guidance identifies an observed executable only as metadata, never as a trust assertion. The user must first verify their original installation, finish its work, use that verified tool's `service stop`, and wait for normal sealing and child exit.

`external_unload_required` means the legacy launchd job remains loaded. PID 0 or “not running” is insufficient proof that the App may unload it. After manually verifying ownership of the current-user job, the user may themselves run:

```sh
/bin/launchctl bootout 'gui/<current-nonroot-UID>/org.openmeetingmemo.service'
```

Replace the UID with the verified current user's numeric UID. This is guidance only: the App does not execute this command for legacy recovery. Do not use sudo or a system domain. Do **not** use `service uninstall` as the precondition for this recovery: it removes the disk plist required below. Bootout leaves that plist, recordings and configuration in place. If identity is uncertain, stop and obtain manual review; do not paste complete `launchctl print` output or credentials into diagnostics.

## Supported stopped recovery

The App accepts only an exact existing Standard/Background LaunchAgent template with matching profile roots/runtime and no extra environment, and only when the current user's job is demonstrably **not loaded**. A bounded, three-second `launchctl print` checks this; stdout is discarded, stderr is bounded and not logged. Only the exact current-user/product job-not-found response with exit 113 is accepted. Permission errors, unknown output, malformed encoding, truncation or timeouts refuse recovery.

Planning never calls `Store::open`, writes owner markers, creates profile lock files, checkpoints SQLite or runs the original CLI. It takes only existing runtime/state lock inodes and performs immutable SQLite reads after rejecting every WAL/SHM/journal sidecar. The exact schema allowlist in `src/application/recovery-schema.json` comes from the checked-in pre-desktop Store schema plus the current desktop tables; it permits missing desktop tables and the historical lazy ASR observation tables, not arbitrary older or future migrations. For the latter, only the exact `asr_measurements` / `asr_measurement_totals` definitions from commit `51319b733cc7c6b01ff39a3a627af95c9da69620` are accepted, with no extra indexes or triggers. Their two ordered autocommit CREATEs allow neither table, the first table alone after interruption, or both; totals without measurements is refused. Observation rows are retained, and these same checks apply to later Start plans for modern owned profiles. Nonzero `user_version`, unknown objects, active leases/work, incomplete commits, recording/unsealed state, reader locks, helper state or retained workspaces refuse admission. Sidecars must be recovered by an appropriate explicit procedure; the App does not delete or ignore them.

1. **Inspect and confirm Switch.** Execution rechecks the expiring plan, roots, bundle/hash, original registration/source identity, configuration, database bytes/inodes, generation and job state. Lock order is lifecycle → registration → runtime → persistent state → profile writer → configuration writer; locks are nonblocking. Only the existing plist's executable is changed to the verified bundle CLI, preserving the original template type and configuration. A private, non-secret plist backup is retained. The owner marker is published atomically with absent-file compare-and-set. This step does not alter DB/configuration/consent bytes, overwrite the personal CLI, bootstrap any job, or start a daemon. Success is **stopped**, not running.
2. **Inspect and confirm Start.** The plan binds the stopped owner/configuration/database/registration snapshot and rechecks it under startup locks. The verified bundled CLI starts through the existing service path. This explicit operation may add supported Store tables. Success requires the actual child PID, bundle identity and capability/generation handshake. A repeated Start cannot start a second daemon. Normal Stop uses the existing atomic idle-shutdown operation and waits for its process to exit.

If registration publication or owner publication fails, compare-and-set restoration preserves the original file where ownership still matches. Unknown concurrent changes are retained, not overwritten. Diagnostics distinguish restored files from a loaded/running job; no old executable is bootstrapped as rollback. Failed Store/preflight initialization does not create or rotate a desktop owner. A legitimately adopted profile remains visibly adopted and stopped if a subsequent Start fails, and can be retried after the cause is resolved.

## Local desktop checks

The native host coordinates related checks and lifecycle controls for the same validated state root within this process. This covers the entire plan (both inspections), execution rechecks and adoption/start, compatibility/info/diagnostics, initialization and offline configuration writes. Independent valid profiles use independent gates. Media streams, file dialogs and unrelated reads are not serialized; diagnostics export releases the gate before the save dialog.

Local waiting is asynchronous, cancellable and limited to five seconds. A local wait timeout returns the existing retryable conflict error, not unknown registration or manual unload guidance. Abandoned waiting operations are not scheduled in the background. Execution after waiting still checks the real plan deadline and current resources; plans retain their original sixty-second lifetime and confirmations are not replayed. Cross-process OS locks remain nonblocking and every original ownership, file identity, sidecar, work and snapshot check remains in place. Direct application/Core callers do not implicitly acquire the native host gate.

## Fixture evidence and limits

`tests/desktop_service_recovery.rs` uses private synthetic roots, a fake service-manager runner, source-bound historical SQL fixtures, synthetic state and a fake notifier. It covers absent owners, same-semver untrusted services with zero RPC connections, read-only plans, schema/sidecar/work refusal, plan expiry/generation/CAS failures, loaded-job refusal, publication failures and explicit adopt → failed Start → retry → running → duplicate-Start refusal → drained Stop. The subprocess case runs only the current CLI with auto-recording and notifications disabled, no pending work, hardware, model or capture peer; after Stop, the real Store measurement writer records a synthetic leased task observation, settles the task, and a second Start/Stop preserves both observation rows and totals. Historical optional-table prefixes preserve database/configuration/sidecar snapshots through planning and stopped adoption; malformed tables, extra indexes/triggers, totals-only, unknown objects and newer schemas remain refused.

`apps/desktop/src-tauri/src/host/recovery_coordination.rs` additionally exercises the production native coordination boundary with controlled async notifications and injected private lifecycle dependencies. It covers automatic Start versus manual Switch/one adoption, actual dispatch of info/diagnostics/preferences after adoption, external-process lock refusal, real snapshot changes, cancellation/timeout/unwind, queued expiry and gate identity through first initialization. Its explicit ignored current-CLI case covers actual private Start/Stop; run it only with `OMM_COORDINATION_TEST_CLI` bound to this revision's built CLI. The controlled lock phase is a deterministic stand-in for a synchronous hash inspection, not a claim about normal timing. The earlier 8 MiB experiment proves contention, not field frequency or the original 09:21 failure's cause.

These fixtures do not demonstrate real personal-service recovery, launchd output on every macOS version, permissions, recording hardware, playback or model/cloud readiness. This change is not an installation or authorization to control an existing personal service.

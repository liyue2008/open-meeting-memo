# Contributor and agent guide

Read [README.md](README.md), [prd.md](prd.md), [the design](docs/overview-design.md) and [installation](docs/install.md) before changing behavior. The CLI baseline is macOS/Apple Silicon + Lark + Pi. The approved Tauri 2 desktop App is now in implementation for Apple Silicon/macOS 15.5+; see [desktop development](docs/desktop.md), [frozen contract](contracts/desktop/v1/README.md), [lane ownership](docs/desktop-lanes.md) and [acceptance](docs/desktop-acceptance.md). Foundation is not a functional desktop release. Windows/Linux, other meeting apps, MCP and browser-served WebUI remain later goals.

## Ownership and seams

- `src/core`: typed contracts, XDG paths/runtime security, configuration, SQLite state, recording lifecycle and persistent pipeline. This layer owns final artifacts and task attempts.
- `src/application`: shared App/CLI use cases and generated desktop DTOs; do not duplicate business rules in Tauri handlers. Freeze `contracts/desktop/v1`, generated Rust/TS and generator under a single internally assigned owner.
- `apps/desktop/src`: React/TypeScript and one typed bridge; no production demo data, content in web storage or unsafe meeting Markdown. `apps/desktop/src-tauri`: thin trusted-window host, deny-default per-command ACL and bounded artifact-only media protocol, no arbitrary paths/shell. Native owns root Cargo/lock and src-tauri; frontend owns npm/lock and UI. See lane board before writes.
- `src/service`: socket control, capture reconciliation, bounded meeting observations and user LaunchAgent registration. Recording must seal before shutdown/uninstall can report success.
- `src/cli`: typed commands, versioned JSON results and exit codes. Diagnostics must not expose content or credentials.
- `src/adapters/meeting`: Lark observations. Unknown is not inactive; window/process signals alone must never trigger automatic recording.
- `src/adapters/capture` and `native/yulu-capture`: the Swift helper and identity-bound socket protocol. **Preserve the imported Yulu audio acquisition/PCM/synchronization/WAV/recovery kernel.** Do not replace it with a new Rust recorder, driver or alternate capture backend. Keep necessary product identity/IPC/packaging fixes narrow, and preserve the MIT license and provenance notice.
- `src/adapters/asr`, `src/bin/asr_worker.rs`, `workers/local-asr`: bounded audio preprocessing and an isolated Rust worker using the pinned sherpa-onnx C API. Keep Rust/C layouts, native lifetimes and model hashes aligned. No Python product runtime or silent engine/model fallback.
- `src/adapters/pi`: transcript-only, bounded Pi print calls and owned workspace cleanup. No inherited personal tools/extensions/context, no audio upload, no new text-upload approval gate.
- `src/core/notifications` and `src/adapters/notification`: generic typed lifecycle/task events and bounded macOS script-host calls. Never send meeting content or treat call success as visible delivery/authorization; no independent OMM notification identity is claimed. Tests must inject a fake notifier or disable notifications, never send desktop notifications implicitly.
- `packaging`: own capture identity, entitlements and immutable model manifest. Never impersonate Yulu or remove dependency licenses.

## Build and controlled validation

Use an existing Rust toolchain and Xcode command-line tools; do not install system tools as a side effect. Desktop additionally uses existing Node 22.19–22.x/npm 11.x and locked project dependencies. Unified entrypoints are `scripts/{dev,test,build,package,install}-desktop.sh`; `test-desktop.sh --foundation` is only a component check. Final acceptance also requires workspace Rust, strict TS/DOM/shared bridge, actual private-profile WKWebView fixtures, release resource/signature verification and real user-only production-package installation/startup.

```sh
cargo fmt --all --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked
bash packaging/capture/build-dev.sh
OMM_CAPTURE_TEST_BINARY="$PWD/target/capture/OMMCapture.app/Contents/MacOS/omm-capture" cargo test --locked --test capture_adapter -- --ignored
OMM_CAPTURE_TEST_BINARY="$PWD/target/capture/OMMCapture.app/Contents/MacOS/omm-capture" cargo test --locked --test core_integration review_r1_03_ -- --ignored
```

The ignored Swift tests use synthetic IPC fixtures, not recording hardware. Do not run all ignored tests indiscriminately; the internal fixture entry point is not a standalone acceptance test. Tests use private temporary roots, controlled helper/worker/Pi processes, synthetic WAVs and a fake C API. None proves real permissions, speech recognition, meeting detection, cloud authentication or long-meeting quality. Keep tests source-named in `tests/`, add focused regression coverage for fixes, and distinguish fixture evidence from real end-to-end results.

## Safety and local installation

- Only explicit user operations install services, download models, request desktop permissions, record or call a real model. Never use personal meetings/chat logs as test material. This desktop development task forbids all audio playback (`play()` included), recording hardware, real notifications, personal content/authentication and real model/cloud calls. Use private fixtures, fake helper/worker/Pi/notifier and synthetic silent WAV metadata/decode/paused seek; never infer playback permission from a recording/install/test authorization.
- Opening a desktop profile is read-only, without implicit SQLite/config migration. All mutations verify current compatible ownership/generation under relevant locks through commit. Legacy/unknown daemon or ownership refuses writes. Offline safe-config CAS is allowed only for explicitly initialized, isolated or provably taken-over new-version profiles, with profile/caller/expiry-bound admission; absent socket alone is not proof. Keep CRUD/reprocessing retention, leases/finite retries and shared config writer locking intact.
- User-only installation: `~/.local/bin`, XDG roots, a user app bundle and `~/Library/LaunchAgents`. No sudo/root, system daemon, virtual driver, global `/usr/local` installation, permission bypass, scripted consent or personal certificate requirement. Ad-hoc signing is development signing, not notarization.
- Inspect existing installation metadata before changing it. Preserve recordings/configuration; back up non-secret metadata before replacement. Do not duplicate secrets in backups, logs or source control. Configure dedicated Pi authentication through Pi's supported mechanisms, not in OMM `config.json`.
- Private OMM directories are 0700 and sensitive files 0600. Preserve canonical-path, symlink, ownership, runtime lock, peer identity and artifact hash checks. macOS runtime fallback is best effort, not a full XDG login-lifecycle guarantee.
- Normal cancellation must wait for owned children before deleting their workspaces. Unknown/live orphan processes are retained and diagnosed, not blindly killed by PID. Uninstall preserves DATA/STATE/CONFIG.
- Keep `Cargo.lock`, native source/headers, model manifests and licenses in version control. Exclude credentials, personal media, weights, runtime state, generated bundles and test transcripts. Do not change the project license incidentally.

Consult [installation and troubleshooting](docs/install.md) for the actual CLI and dependency steps. Report remaining permission/network/authentication blockers honestly; do not call a successful registration, mock test or unknown permission state “ready”.

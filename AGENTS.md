# Contributor and agent guide

Read [README.md](README.md), [prd.md](prd.md) and [the design](docs/overview-design.md) before changing behavior. The first implementation is macOS/Apple Silicon + Lark + Pi + CLI. Windows/Linux, other meeting apps, MCP and WebUI are later goals, not implemented features.

## Ownership and seams

- `src/core`: typed contracts, XDG paths/runtime security, configuration, SQLite state, recording lifecycle and persistent pipeline. This layer owns final artifacts and task attempts.
- `src/service`: socket control, capture reconciliation, bounded meeting observations and user LaunchAgent registration. Recording must seal before shutdown/uninstall can report success.
- `src/cli`: typed commands, versioned JSON results and exit codes. Diagnostics must not expose content or credentials.
- `src/adapters/meeting`: Lark observations. Unknown is not inactive; window/process signals alone must never trigger automatic recording.
- `src/adapters/capture` and `native/yulu-capture`: the Swift helper and identity-bound socket protocol. **Preserve the imported Yulu audio acquisition/PCM/synchronization/WAV/recovery kernel.** Do not replace it with a new Rust recorder, driver or alternate capture backend. Keep necessary product identity/IPC/packaging fixes narrow, and preserve the MIT license and provenance notice.
- `src/adapters/asr`, `src/bin/asr_worker.rs`, `workers/local-asr`: bounded audio preprocessing and an isolated Rust worker using the pinned sherpa-onnx C API. Keep Rust/C layouts, native lifetimes and model hashes aligned. No Python product runtime or silent engine/model fallback.
- `src/adapters/pi`: transcript-only, bounded Pi print calls and owned workspace cleanup. No inherited personal tools/extensions/context, no audio upload, no new text-upload approval gate.
- `packaging`: own capture identity, entitlements and immutable model manifest. Never impersonate Yulu or remove dependency licenses.

## Build and controlled validation

Use an existing Rust toolchain and Xcode command-line tools; do not install system tools as a side effect.

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

- Only explicit user operations install services, download models, request desktop permissions, record or call a real model. Never use personal meetings/chat logs as test material. Use short, self-generated speech and consented recording; cloud tests send only the generated transcript.
- User-only installation: `~/.local/bin`, XDG roots, a user app bundle and `~/Library/LaunchAgents`. No sudo/root, system daemon, virtual driver, global `/usr/local` installation, permission bypass, scripted consent or personal certificate requirement. Ad-hoc signing is development signing, not notarization.
- Inspect existing installation metadata before changing it. Preserve recordings/configuration; back up non-secret metadata before replacement. Do not duplicate secrets in backups, logs or source control. Configure dedicated Pi authentication through Pi's supported mechanisms, not in OMM `config.json`.
- Private OMM directories are 0700 and sensitive files 0600. Preserve canonical-path, symlink, ownership, runtime lock, peer identity and artifact hash checks. macOS runtime fallback is best effort, not a full XDG login-lifecycle guarantee.
- Normal cancellation must wait for owned children before deleting their workspaces. Unknown/live orphan processes are retained and diagnosed, not blindly killed by PID. Uninstall preserves DATA/STATE/CONFIG.
- Keep `Cargo.lock`, native source/headers, model manifests and licenses in version control. Exclude credentials, personal media, weights, runtime state, generated bundles and test transcripts. Do not change the project license incidentally.

Consult [installation and troubleshooting](docs/install.md) for the actual CLI and dependency steps. Report remaining permission/network/authentication blockers honestly; do not call a successful registration, mock test or unknown permission state “ready”.

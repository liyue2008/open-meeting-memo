# Capture operational diagnostics

The helper keeps imported free-form log messages redacted. Operational events use
an internal fixed enum and optional signed numeric code; never paths, titles,
device UIDs, session IDs, raw error text or audio samples. This does not change the
public IPC or sealed descriptor schema.

- `tap_create_success`: code 1 is a permission probe, 0 is a capture build. A
  successful HAL creation does **not** prove nonzero audio was received.
- `tap_create_error`, `aggregate_create_error`, `io_proc_create_error`,
  `device_start_error`: numeric OSStatus from that operation.
- `system_nonzero_seen`: first accepted nonzero system buffer per recording,
  emitted on the recorder queue, not once per audio frame.
- `zero_recovery`: existing recovery policy tripped; code is the zero callback
  run length. `zero_recovery_success` / `zero_recovery_failure` describe its
  existing bounded rebuild result. No recovery policy was changed.
- `mic_gate`: meeting-state transition while recording; 1 muted, 0 unmuted,
  -1 unknown. Unknown still records; repeated identical states do not log.
- `silence_threshold_reached`: existing dual-source silence threshold crossed,
  once per pending stop request. This alone does not prove recording stopped.
- `stop_silence`: the existing silence callback proceeds past its confirmation
  path to stopping. `stop_requested` means an active recorder was asked to stop,
  whether manually or automatically; repeated inactive stops do not log.

These diagnostics do not repair silent capture, establish historical permissions,
or recover missing sound. The 300-second dual-source silence policy is unchanged.

Run only synthetic entry points (no AppDelegate, capture device or playback):

```sh
bash packaging/capture/build-dev.sh
OMM_CAPTURE_TEST_BINARY="$PWD/target/capture/OMMCapture.app/Contents/MacOS/omm-capture" \
  python3 packaging/capture/test_operational_events.py
```

The tests use generated nonzero PCM for 16/44.1/48 kHz conversion, the actual
system IO handler with injected buffers, route/zero-recovery policy, mic gating,
the exact 300-second silence boundary, and bounded/redacted event output.
They do not test real Node/Pi, TCC consent, hardware capture or audible playback.

The Python driver allows 30 seconds per native fixture, then at most 5 seconds
for a second wait/drain after timeout, interruption or communication failure.
It sends no signals. A failed invocation always retains its private workspace
and `receipt.json`; successful cleanup requires a real child wait and both pipe
EOFs. If the bounded drain cannot confirm completion, the receipt stays
`waited=false`, and the actual Popen handle is retained in the driver instead
of treating PID disappearance as a wait. Do not delete that workspace while the
child may still be live. Ownership regressions use only finite Python children
and explicitly wait/drain them before their test directories are removed.

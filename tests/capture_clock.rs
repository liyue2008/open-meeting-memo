use omm::core::contracts::{deserialize_capture_clock, CaptureClock, PiConfig};
use serde::Deserialize;
use serde_json::{json, Value};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptorClock {
    #[serde(default, deserialize_with = "deserialize_capture_clock")]
    capture_clock: Option<CaptureClock>,
}
fn clock(rate: f64) -> Value {
    json!({"schema_version":1,"target_rate_hz":48000,"writer_started_utc_ms":1,"elapsed_us":2000,"format_rejects":0,"mic":{"rate_hz":rate,"input_frames":48,"target_frames":48,"rate_changes":0,"accepted_channels":1,"input_elapsed_us":1000,"first_accepted_offset_us":1,"last_accepted_offset_us":1001}})
}
#[test]
fn capture_clock_45_known_rate_source_layouts_and_legacy_remain_valid() {
    assert!(serde_json::from_value::<DescriptorClock>(json!({}))
        .unwrap()
        .capture_clock
        .is_none());
    for rate in [8000.0, 16000.0, 24000.0, 44100.0, 48000.0] {
        for source in 0..3 {
            for changes in 0..3 {
                let mut c = clock(rate);
                c["mic"]["rate_changes"] = json!(changes);
                if source > 0 {
                    c["system"] = c["mic"].clone();
                    c["system"]["accepted_channels"] = json!(2);
                }
                if source == 2 {
                    c.as_object_mut().unwrap().remove("mic");
                }
                assert!(
                    serde_json::from_value::<DescriptorClock>(json!({"capture_clock":c}))
                        .unwrap()
                        .capture_clock
                        .unwrap()
                        .valid()
                );
            }
        }
    }
}
#[test]
fn capture_clock_present_invalid_null_unknown_or_unsupported_is_not_legacy() {
    for (path, value) in [
        ("/schema_version", json!(2)),
        ("/target_rate_hz", json!(44100)),
        ("/mic/rate_hz", json!(-1)),
        ("/mic/accepted_channels", json!(2)),
        ("/mic/input_elapsed_us", json!(1001)),
        ("/mic/last_accepted_offset_us", json!(999999)),
    ] {
        let mut c = clock(44100.0);
        *c.pointer_mut(path).unwrap() = value;
        assert!(serde_json::from_value::<DescriptorClock>(json!({"capture_clock":c})).is_err());
    }
    for c in [
        Value::Null,
        json!({}),
        {
            let mut c = clock(48000.0);
            c["unknown"] = json!(1);
            c
        },
        {
            let mut c = clock(48000.0);
            c["mic"]["secret_field"] = json!(1);
            c
        },
    ] {
        assert!(serde_json::from_value::<DescriptorClock>(json!({"capture_clock":c})).is_err());
    }
    assert!(serde_json::from_value::<DescriptorClock>(json!({"other":1})).is_err());
}
#[test]
fn capture_clock_idle_and_started_before_first_buffer_are_valid_status_shapes() {
    for (utc, elapsed) in [(0, 0), (1, 0), (1, 2000)] {
        let c = json!({"schema_version":1,"target_rate_hz":48000,"writer_started_utc_ms":utc,"elapsed_us":elapsed,"format_rejects":0});
        assert!(serde_json::from_value::<DescriptorClock>(json!({"capture_clock":c})).is_ok());
    }
    let mut c = clock(44100.0);
    c["writer_started_utc_ms"] = json!(0);
    assert!(serde_json::from_value::<DescriptorClock>(json!({"capture_clock":c})).is_err());
}
#[test]
fn old_pi_snapshot_absent_runtime_roundtrips_without_changing_fingerprint() {
    // Exact pre-runtime-field wire bytes: old task fingerprints must remain byte-identical.
    let old = br#"{"executable":"/trusted/pi","provider":"fixture","model":"fixed","prompt_version":"v1","timeout_ms":1000,"max_input_bytes":2000,"max_output_bytes":3000}"#;
    let parsed: PiConfig = serde_json::from_slice(old).unwrap();
    assert_eq!(serde_json::to_vec(&parsed).unwrap(), old);
    let mut v = serde_json::to_value(parsed).unwrap();
    assert!(v.get("node_executable").is_none());
    v["node_executable"] = json!("/trusted/node");
    assert!(serde_json::from_value::<PiConfig>(v)
        .unwrap()
        .node_executable
        .is_some());
    let mut config = omm::core::config::AppConfig::default();
    config.set("pi.node_executable", "/trusted/node").unwrap();
    assert_eq!(
        config.pi.node_executable.unwrap().to_str(),
        Some("/trusted/node")
    );
    config = omm::core::config::AppConfig::default();
    config.set("pi.node_executable", "/trusted/node").unwrap();
    config.set("pi.node_executable", "null").unwrap();
    assert!(config.pi.node_executable.is_none());
}

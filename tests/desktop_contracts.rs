//! Structural serde checks paired with the JSON Schema boundary tests in the desktop project.
use omm::application::desktop_contracts::{
    ConfigPatch, DesktopRequest, DesktopResponse, MeetingRow,
};
use serde_json::{json, Value};
use std::{fs, path::Path};

#[test]
fn desktop_contracts_shared_examples_roundtrip_without_loss() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("contracts/desktop/v1/examples");
    let mut count = 0;
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let document: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let examples = match document {
            Value::Array(items) => items,
            item => vec![item],
        };
        for example in examples {
            let request = json!({"command":example["command"], "request":example["request"]});
            let response = json!({"command":example["command"], "response":example["response"]});
            let parsed: DesktopRequest = serde_json::from_value(request.clone()).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), request);
            let parsed: DesktopResponse = serde_json::from_value(response.clone()).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), response);
            let mut unknown = request.clone();
            unknown["request"]["payload"]["unexpected"] = json!(true);
            assert!(serde_json::from_value::<DesktopRequest>(unknown).is_err());
            let mut unknown = response;
            unknown["response"]["unexpected"] = json!(true);
            assert!(serde_json::from_value::<DesktopResponse>(unknown).is_err());
            count += 1;
        }
    }
    assert_eq!(
        count, 48,
        "36 commands, 10 core error codes and 2 admission modes"
    );
}

#[test]
fn desktop_contracts_config_patch_distinguishes_omitted_null_and_value() {
    let omitted: ConfigPatch = serde_json::from_value(json!({})).unwrap();
    let null: ConfigPatch = serde_json::from_value(json!({"piNodeExecutable": null})).unwrap();
    let value: ConfigPatch =
        serde_json::from_value(json!({"piNodeExecutable": "/fixture/bin/node"})).unwrap();
    assert_eq!(omitted.pi_node_executable, None);
    assert_eq!(null.pi_node_executable, Some(None));
    assert_eq!(
        value.pi_node_executable,
        Some(Some("/fixture/bin/node".into()))
    );
    assert_eq!(
        serde_json::to_value(null).unwrap(),
        json!({"piNodeExecutable": null})
    );
    assert!(serde_json::from_value::<ConfigPatch>(json!({"autoRecord":true})).is_err());
    assert!(serde_json::from_value::<ConfigPatch>(json!({"apiKey":"forbidden"})).is_err());
}

#[test]
fn desktop_contracts_required_nullable_fields_cannot_be_omitted() {
    let example: Value = serde_json::from_str(include_str!(
        "../contracts/desktop/v1/examples/meeting_update.json"
    ))
    .unwrap();
    let mut row = example["response"]["data"].clone();
    assert!(serde_json::from_value::<MeetingRow>(row.clone()).is_ok());
    row.as_object_mut().unwrap().remove("startedAtMs");
    assert!(serde_json::from_value::<MeetingRow>(row).is_err());
}

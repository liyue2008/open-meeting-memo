//! Runtime validation of the frozen shared schema, including budgets omitted by serde.
use crate::core::contracts::*;
use serde_json::Value;
use std::sync::OnceLock;
fn schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        serde_json::from_str(include_str!("../../contracts/desktop/v1/schema.json"))
            .expect("versioned desktop schema")
    })
}
fn invalid() -> OmmError {
    OmmError::invalid("desktop request violates the versioned contract")
}
pub fn validate_named(name: &str, value: &Value) -> Result<()> {
    validate(&schema()["$defs"][name], value, 0)
}
fn validate(rule: &Value, value: &Value, depth: usize) -> Result<()> {
    if depth > 32 || rule.is_null() {
        return Err(invalid());
    }
    if let Some(reference) = rule["$ref"].as_str() {
        return validate_named(
            reference.strip_prefix("#/$defs/").ok_or_else(invalid)?,
            value,
        );
    }
    if let Some(choices) = rule["anyOf"].as_array() {
        return if choices
            .iter()
            .any(|r| validate(r, value, depth + 1).is_ok())
        {
            Ok(())
        } else {
            Err(invalid())
        };
    }
    if rule.get("const").is_some_and(|expected| expected != value)
        || rule["enum"]
            .as_array()
            .is_some_and(|choices| !choices.contains(value))
    {
        return Err(invalid());
    }
    match rule["type"].as_str() {
        Some("null") if !value.is_null() => return Err(invalid()),
        Some("boolean") if !value.is_boolean() => return Err(invalid()),
        Some("integer") => {
            let n = value.as_i64().ok_or_else(invalid)?;
            if n.unsigned_abs() > 9_007_199_254_740_991
                || rule["minimum"].as_i64().is_some_and(|min| n < min)
                || rule["maximum"].as_i64().is_some_and(|max| n > max)
            {
                return Err(invalid());
            }
        }
        Some("string") => {
            let text = value.as_str().ok_or_else(invalid)?;
            let len = text.chars().count() as u64;
            if rule["minLength"].as_u64().is_some_and(|min| len < min)
                || rule["maxLength"].as_u64().is_some_and(|max| len > max)
            {
                return Err(invalid());
            }
            if rule["format"] == "uuid" && uuid::Uuid::parse_str(text).is_err() {
                return Err(invalid());
            }
            if let Some(pattern) = rule["pattern"].as_str() {
                let valid = match pattern {
                    "^[a-f0-9]{64}$" => {
                        text.len() == 64
                            && text
                                .bytes()
                                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                    }
                    "^[A-Za-z0-9_.:-]{1,128}$" => {
                        !text.is_empty()
                            && text.len() <= 128
                            && text
                                .bytes()
                                .all(|c| c.is_ascii_alphanumeric() || b"_.:-".contains(&c))
                    }
                    _ => true,
                };
                if !valid {
                    return Err(invalid());
                }
            }
        }
        Some("array") => {
            let values = value.as_array().ok_or_else(invalid)?;
            if rule["minItems"]
                .as_u64()
                .is_some_and(|min| (values.len() as u64) < min)
                || rule["maxItems"]
                    .as_u64()
                    .is_some_and(|max| values.len() as u64 > max)
            {
                return Err(invalid());
            }
            for (i, item) in values.iter().enumerate() {
                if rule["uniqueItems"] == true && values[..i].contains(item) {
                    return Err(invalid());
                }
                validate(&rule["items"], item, depth + 1)?;
            }
        }
        Some("object") => {
            let object = value.as_object().ok_or_else(invalid)?;
            if let Some(required) = rule["required"].as_array() {
                if required
                    .iter()
                    .any(|key| !object.contains_key(key.as_str().unwrap_or("")))
                {
                    return Err(invalid());
                }
            }
            for (key, value) in object {
                if rule["properties"].get(key).is_none() {
                    if rule["additionalProperties"] == false {
                        return Err(invalid());
                    }
                } else {
                    validate(&rule["properties"][key], value, depth + 1)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn request(command: &str, request: &Value) -> Result<()> {
    if serde_json::to_vec(request)?.len() > 128 * 1024 {
        return Err(OmmError::invalid("desktop request exceeds budget"));
    }
    validate_named(&format!("{command}_request"), request)?;
    let payload = &request["payload"];
    if payload["metadata"]["note"]
        .as_str()
        .is_some_and(|note| note.len() > 16 * 1024)
    {
        return Err(OmmError::invalid("meeting note exceeds UTF-8 budget"));
    }
    if let Some(date) = payload["filter"]["date"].as_object() {
        use chrono::{NaiveDate, TimeZone};
        let zone: chrono_tz::Tz = date["timeZone"]
            .as_str()
            .ok_or_else(invalid)?
            .parse()
            .map_err(|_| invalid())?;
        let boundary = |field: &str| -> Result<i64> {
            let date =
                NaiveDate::parse_from_str(date[field].as_str().ok_or_else(invalid)?, "%Y-%m-%d")
                    .map_err(|_| invalid())?;
            zone.from_local_datetime(&date.and_hms_opt(0, 0, 0).ok_or_else(invalid)?)
                .earliest()
                .map(|time| time.timestamp_millis())
                .ok_or_else(invalid)
        };
        let start = boundary("fromLocalDate")?;
        let end = boundary("toLocalDateExclusive")?;
        if start >= end || date["fromUtcMs"] != start || date["toUtcMsExclusive"] != end {
            return Err(OmmError::invalid(
                "date boundaries do not match the selected time zone",
            ));
        }
    }
    Ok(())
}
pub fn response(command: &str, response: &Value) -> Result<()> {
    if serde_json::to_vec(response)?.len() > 1024 * 1024 {
        return Err(OmmError::new(
            ErrorCode::Unavailable,
            false,
            "desktop response exceeds budget; use a smaller page",
        ));
    }
    let name = schema()["x-commands"][command]["response"]
        .as_str()
        .ok_or_else(invalid)?;
    validate_named(name, response)
}

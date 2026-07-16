//! Declarative measurement for typed structured JSON records.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use calyx_aster::dedup::IngestInput;
use calyx_core::{AbsentReason, CalyxError, Input, LensId, Modality, Result, SlotId, SlotVector};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use sha2::{Digest, Sha256};

use crate::Registry;

pub const CALYX_STRUCTURED_SCHEMA_INVALID: &str = "CALYX_STRUCTURED_SCHEMA_INVALID";
pub const CALYX_STRUCTURED_RECORD_INVALID: &str = "CALYX_STRUCTURED_RECORD_INVALID";
pub const CALYX_STRUCTURED_FIELD_MISSING: &str = "CALYX_STRUCTURED_FIELD_MISSING";
pub const CALYX_STRUCTURED_FIELD_TYPE_MISMATCH: &str = "CALYX_STRUCTURED_FIELD_TYPE_MISMATCH";

const STRUCTURED_REMEDIATION: &str =
    "fix the structured record schema or source row and retry without partial ingest";
const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;

/// Declarative field-to-Calyx mapping for one structured record domain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredRecordSchema {
    pub domain: String,
    pub panel_version: u32,
    pub fields: Vec<StructuredFieldSchema>,
}

impl StructuredRecordSchema {
    /// Validates schema shape before any record in a batch is measured.
    pub fn validate(&self) -> Result<()> {
        if self.domain.trim().is_empty() {
            return Err(structured_error(
                CALYX_STRUCTURED_SCHEMA_INVALID,
                "structured record schema domain must not be empty",
            ));
        }
        if self.panel_version == 0 {
            return Err(structured_error(
                CALYX_STRUCTURED_SCHEMA_INVALID,
                "structured record schema panel_version must be greater than zero",
            ));
        }
        if self.fields.is_empty() {
            return Err(structured_error(
                CALYX_STRUCTURED_SCHEMA_INVALID,
                "structured record schema must declare at least one field",
            ));
        }

        let mut slots = BTreeSet::new();
        let mut scalars = BTreeSet::new();
        let mut metadata = BTreeSet::new();
        for field in &self.fields {
            field.validate()?;
            for slot in &field.slots {
                if !slots.insert(slot.slot_id) {
                    return Err(structured_error(
                        CALYX_STRUCTURED_SCHEMA_INVALID,
                        format!(
                            "structured schema declares slot {} more than once",
                            slot.slot_id
                        ),
                    ));
                }
            }
            if let Some(route) = &field.scalar {
                if !matches!(
                    field.value_type,
                    StructuredFieldType::Number | StructuredFieldType::Integer
                ) {
                    return Err(structured_error(
                        CALYX_STRUCTURED_SCHEMA_INVALID,
                        format!(
                            "field {} routes to scalar {:?} but is not numeric",
                            field.path, route.key
                        ),
                    ));
                }
                if !scalars.insert(route.key.as_str()) {
                    return Err(structured_error(
                        CALYX_STRUCTURED_SCHEMA_INVALID,
                        format!(
                            "structured schema declares scalar {:?} more than once",
                            route.key
                        ),
                    ));
                }
            }
            if let Some(route) = &field.metadata
                && !metadata.insert(route.key.as_str())
            {
                return Err(structured_error(
                    CALYX_STRUCTURED_SCHEMA_INVALID,
                    format!(
                        "structured schema declares metadata {:?} more than once",
                        route.key
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// One field extracted by RFC 6901 JSON Pointer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredFieldSchema {
    pub path: String,
    pub value_type: StructuredFieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub slots: Vec<StructuredSlotMeasure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scalar: Option<StructuredScalarRoute>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<StructuredMetadataRoute>,
}

impl StructuredFieldSchema {
    fn validate(&self) -> Result<()> {
        validate_json_pointer(&self.path)?;
        if self.slots.is_empty() && self.scalar.is_none() && self.metadata.is_none() {
            return Err(structured_error(
                CALYX_STRUCTURED_SCHEMA_INVALID,
                format!("field {} has no slot, scalar, or metadata route", self.path),
            ));
        }
        let mut slot_ids = BTreeSet::new();
        for slot in &self.slots {
            if !slot_ids.insert(slot.slot_id) {
                return Err(structured_error(
                    CALYX_STRUCTURED_SCHEMA_INVALID,
                    format!("field {} repeats slot {}", self.path, slot.slot_id),
                ));
            }
        }
        if let Some(route) = &self.scalar {
            validate_route_key("scalar", &route.key)?;
        }
        if let Some(route) = &self.metadata {
            validate_route_key("metadata", &route.key)?;
        }
        Ok(())
    }
}

/// Expected JSON field type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredFieldType {
    Any,
    String,
    Number,
    Integer,
    Boolean,
    Object,
    Array,
}

/// Per-field lens route. The lens is measured on typed field bytes and stored
/// under the declared slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredSlotMeasure {
    pub slot_id: SlotId,
    pub lens_id: LensId,
}

/// Exact f64 scalar route for a numeric field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredScalarRoute {
    pub key: String,
}

/// Verbatim metadata route for a field. Strings are stored without quotes;
/// non-string values use the same deterministic canonical JSON bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredMetadataRoute {
    pub key: String,
}

/// Fully measured batch. No vault writes have happened yet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredBatchMeasurement {
    pub schema_domain: String,
    pub panel_version: u32,
    pub records: Vec<StructuredRecordMeasurement>,
}

/// One measured structured record and its deterministic byte evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredRecordMeasurement {
    pub index: usize,
    pub canonical_sha256: [u8; 32],
    pub input: IngestInput,
}

/// Measures a fail-closed batch of structured records. The caller may persist
/// the returned [`IngestInput`] values only after this function returns `Ok`.
pub fn measure_structured_record_batch(
    registry: &Registry,
    schema: &StructuredRecordSchema,
    records: &[Value],
) -> Result<StructuredBatchMeasurement> {
    schema.validate()?;
    let mut measured = Vec::with_capacity(records.len());
    for (index, record) in records.iter().enumerate() {
        measured.push(measure_structured_record_inner(
            registry, schema, record, index,
        )?);
    }
    Ok(StructuredBatchMeasurement {
        schema_domain: schema.domain.clone(),
        panel_version: schema.panel_version,
        records: measured,
    })
}

/// Measures one structured record with index `0`.
pub fn measure_structured_record(
    registry: &Registry,
    schema: &StructuredRecordSchema,
    record: &Value,
) -> Result<StructuredRecordMeasurement> {
    schema.validate()?;
    measure_structured_record_inner(registry, schema, record, 0)
}

/// Deterministic compact JSON bytes with recursive object-key ordering.
pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    write_canonical_json(value, &mut out)?;
    Ok(out)
}

fn measure_structured_record_inner(
    registry: &Registry,
    schema: &StructuredRecordSchema,
    record: &Value,
    index: usize,
) -> Result<StructuredRecordMeasurement> {
    if !record.is_object() {
        return Err(structured_error(
            CALYX_STRUCTURED_RECORD_INVALID,
            format!("record {index} is not a JSON object"),
        ));
    }
    let canonical = canonical_json_bytes(record)?;
    let canonical_sha256 = Sha256::digest(&canonical).into();
    let mut input = IngestInput::new(canonical, schema.panel_version, Modality::Structured);
    input.redacted = false;
    input
        .metadata
        .insert("structured_domain".to_owned(), schema.domain.clone());
    input.metadata.insert(
        "structured_schema_panel_version".to_owned(),
        schema.panel_version.to_string(),
    );

    for field in &schema.fields {
        let value = record.pointer(&field.path);
        match value {
            Some(Value::Null) | None => {
                if field.required {
                    return Err(structured_error(
                        CALYX_STRUCTURED_FIELD_MISSING,
                        format!("record {index} missing required field {}", field.path),
                    ));
                }
                for route in &field.slots {
                    input.slots.insert(
                        route.slot_id,
                        SlotVector::Absent {
                            reason: AbsentReason::Error(format!(
                                "structured_field_missing:{}",
                                field.path
                            )),
                        },
                    );
                }
            }
            Some(value) => {
                ensure_field_type(index, field, value)?;
                if let Some(route) = &field.scalar {
                    let scalar = number_to_exact_f64(index, &field.path, value)?;
                    input.scalars.insert(route.key.clone(), scalar);
                }
                if let Some(route) = &field.metadata {
                    input
                        .metadata
                        .insert(route.key.clone(), metadata_value(value)?);
                }
                if !field.slots.is_empty() {
                    let field_bytes = field_measurement_bytes(value)?;
                    let field_input = Input::new(Modality::Structured, field_bytes);
                    for route in &field.slots {
                        let vector = registry.measure(route.lens_id, &field_input)?;
                        input.slots.insert(route.slot_id, vector);
                    }
                }
            }
        }
    }

    Ok(StructuredRecordMeasurement {
        index,
        canonical_sha256,
        input,
    })
}

fn ensure_field_type(index: usize, field: &StructuredFieldSchema, value: &Value) -> Result<()> {
    let matches = match field.value_type {
        StructuredFieldType::Any => true,
        StructuredFieldType::String => value.is_string(),
        StructuredFieldType::Number => value.as_number().is_some_and(json_number_is_finite),
        StructuredFieldType::Integer => value.as_number().is_some_and(json_number_is_integer),
        StructuredFieldType::Boolean => value.is_boolean(),
        StructuredFieldType::Object => value.is_object(),
        StructuredFieldType::Array => value.is_array(),
    };
    if matches {
        return Ok(());
    }
    Err(structured_error(
        CALYX_STRUCTURED_FIELD_TYPE_MISMATCH,
        format!(
            "record {index} field {} expected {:?}, got {}",
            field.path,
            field.value_type,
            json_type_name(value)
        ),
    ))
}

fn number_to_exact_f64(index: usize, path: &str, value: &Value) -> Result<f64> {
    let Some(number) = value.as_number() else {
        return Err(structured_error(
            CALYX_STRUCTURED_FIELD_TYPE_MISMATCH,
            format!("record {index} field {path} is not numeric"),
        ));
    };
    if let Some(raw) = number.as_u64()
        && raw > MAX_SAFE_JSON_INTEGER
    {
        return Err(structured_error(
            CALYX_STRUCTURED_FIELD_TYPE_MISMATCH,
            format!(
                "record {index} field {path} integer {raw} exceeds exact f64 JSON scalar range"
            ),
        ));
    }
    if let Some(raw) = number.as_i64()
        && raw.unsigned_abs() > MAX_SAFE_JSON_INTEGER
    {
        return Err(structured_error(
            CALYX_STRUCTURED_FIELD_TYPE_MISMATCH,
            format!(
                "record {index} field {path} integer {raw} exceeds exact f64 JSON scalar range"
            ),
        ));
    }
    let Some(value) = number.as_f64() else {
        return Err(structured_error(
            CALYX_STRUCTURED_FIELD_TYPE_MISMATCH,
            format!("record {index} field {path} cannot be represented as f64"),
        ));
    };
    if value.is_finite() {
        Ok(value)
    } else {
        Err(structured_error(
            CALYX_STRUCTURED_FIELD_TYPE_MISMATCH,
            format!("record {index} field {path} is NaN or Inf"),
        ))
    }
}

fn metadata_value(value: &Value) -> Result<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        _ => String::from_utf8(canonical_json_bytes(value)?).map_err(|error| {
            structured_error(
                CALYX_STRUCTURED_RECORD_INVALID,
                format!("canonical metadata value was not UTF-8: {error}"),
            )
        }),
    }
}

fn field_measurement_bytes(value: &Value) -> Result<Vec<u8>> {
    match value {
        Value::String(value) => Ok(value.as_bytes().to_vec()),
        _ => canonical_json_bytes(value),
    }
}

fn write_canonical_json(value: &Value, out: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(number) => {
            if !json_number_is_finite(number) {
                return Err(structured_error(
                    CALYX_STRUCTURED_RECORD_INVALID,
                    "JSON number is not finite",
                ));
            }
            out.extend_from_slice(number.to_string().as_bytes());
        }
        Value::String(value) => serde_json::to_writer(out, value).map_err(|error| {
            structured_error(
                CALYX_STRUCTURED_RECORD_INVALID,
                format!("serialize canonical JSON string: {error}"),
            )
        })?,
        Value::Array(values) => {
            out.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_canonical_json(value, out)?;
            }
            out.push(b']');
        }
        Value::Object(values) => {
            out.push(b'{');
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| compare_json_keys_utf16(left, right));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, key).map_err(|error| {
                    structured_error(
                        CALYX_STRUCTURED_RECORD_INVALID,
                        format!("serialize canonical JSON object key: {error}"),
                    )
                })?;
                out.push(b':');
                write_canonical_json(value, out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

fn compare_json_keys_utf16(left: &str, right: &str) -> Ordering {
    left.encode_utf16().cmp(right.encode_utf16())
}

fn json_number_is_finite(number: &Number) -> bool {
    number.as_f64().is_some_and(f64::is_finite)
}

fn json_number_is_integer(number: &Number) -> bool {
    number.as_i64().is_some() || number.as_u64().is_some()
}

fn validate_json_pointer(pointer: &str) -> Result<()> {
    if pointer.is_empty() {
        return Ok(());
    }
    if !pointer.starts_with('/') {
        return Err(structured_error(
            CALYX_STRUCTURED_SCHEMA_INVALID,
            format!("JSON Pointer {pointer:?} must be empty or start with '/'"),
        ));
    }
    for token in pointer.split('/').skip(1) {
        let mut chars = token.chars();
        while let Some(ch) = chars.next() {
            if ch == '~' {
                match chars.next() {
                    Some('0' | '1') => {}
                    _ => {
                        return Err(structured_error(
                            CALYX_STRUCTURED_SCHEMA_INVALID,
                            format!("JSON Pointer {pointer:?} contains invalid '~' escape"),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_route_key(kind: &str, key: &str) -> Result<()> {
    if key.is_empty() {
        return Err(structured_error(
            CALYX_STRUCTURED_SCHEMA_INVALID,
            format!("{kind} route key must not be empty"),
        ));
    }
    Ok(())
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn structured_error(code: &'static str, message: impl Into<String>) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation: STRUCTURED_REMEDIATION,
    }
}

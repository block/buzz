//! AgentMemory's CPython 3.11-compatible canonical JSON encoding.

use serde_json::Value;

const MAXIMUM_JSON_DEPTH: usize = 64;
const MAXIMUM_JSON_NODES: usize = 10_000;

/// Failure while encoding bounded AgentMemory canonical JSON.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CanonicalJsonError {
    /// The value exceeded the agreed byte, depth, or node bound.
    BoundsExceeded,
    /// A JSON number or string could not be represented by the contract.
    InvalidValue,
}

/// Encodes a JSON value exactly like AgentMemory's CPython 3.11
/// `json.dumps(..., ensure_ascii=False, allow_nan=False, separators=(",", ":"),
/// sort_keys=True)`, subject to explicit structural and byte bounds.
pub fn canonical_json_bytes(
    value: &Value,
    maximum_bytes: usize,
) -> Result<Vec<u8>, CanonicalJsonError> {
    fn write(
        value: &Value,
        output: &mut Vec<u8>,
        maximum_bytes: usize,
        depth: usize,
        nodes: &mut usize,
    ) -> Result<(), CanonicalJsonError> {
        if depth > MAXIMUM_JSON_DEPTH || *nodes >= MAXIMUM_JSON_NODES {
            return Err(CanonicalJsonError::BoundsExceeded);
        }
        *nodes += 1;
        match value {
            Value::Null => output.extend_from_slice(b"null"),
            Value::Bool(true) => output.extend_from_slice(b"true"),
            Value::Bool(false) => output.extend_from_slice(b"false"),
            Value::Number(number) => {
                let encoded = if number.as_i64().is_some() || number.as_u64().is_some() {
                    number.to_string()
                } else {
                    python_float(number)?
                };
                output.extend_from_slice(encoded.as_bytes());
            }
            Value::String(text) => output.extend_from_slice(
                serde_json::to_string(text)
                    .map_err(|_| CanonicalJsonError::InvalidValue)?
                    .as_bytes(),
            ),
            Value::Array(values) => {
                output.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    write(value, output, maximum_bytes, depth + 1, nodes)?;
                }
                output.push(b']');
            }
            Value::Object(values) => {
                let mut entries = values.iter().collect::<Vec<_>>();
                entries.sort_by(|left, right| left.0.cmp(right.0));
                output.push(b'{');
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    output.extend_from_slice(
                        serde_json::to_string(key)
                            .map_err(|_| CanonicalJsonError::InvalidValue)?
                            .as_bytes(),
                    );
                    output.push(b':');
                    write(value, output, maximum_bytes, depth + 1, nodes)?;
                }
                output.push(b'}');
            }
        }
        if output.len() > maximum_bytes {
            return Err(CanonicalJsonError::BoundsExceeded);
        }
        Ok(())
    }

    if maximum_bytes == 0 {
        return Err(CanonicalJsonError::BoundsExceeded);
    }
    let mut output = Vec::new();
    let mut nodes = 0;
    write(value, &mut output, maximum_bytes, 0, &mut nodes)?;
    Ok(output)
}

fn python_float(number: &serde_json::Number) -> Result<String, CanonicalJsonError> {
    let rendered = number.to_string();
    let (sign, magnitude) = rendered
        .strip_prefix('-')
        .map_or(("", rendered.as_str()), |value| ("-", value));
    let (coefficient, explicit_exponent) = magnitude
        .split_once(['e', 'E'])
        .map_or((magnitude, 0_i32), |(coefficient, exponent)| {
            (coefficient, exponent.parse::<i32>().unwrap_or(i32::MIN))
        });
    if explicit_exponent == i32::MIN {
        return Err(CanonicalJsonError::InvalidValue);
    }
    let decimal_position = coefficient.find('.').unwrap_or(coefficient.len()) as i32;
    let digits = coefficient
        .bytes()
        .filter(|byte| *byte != b'.')
        .collect::<Vec<_>>();
    let Some(first_nonzero) = digits.iter().position(|byte| *byte != b'0') else {
        return Ok(format!("{sign}0.0"));
    };
    let last_nonzero = digits
        .iter()
        .rposition(|byte| *byte != b'0')
        .ok_or(CanonicalJsonError::InvalidValue)?;
    let significant = std::str::from_utf8(&digits[first_nonzero..=last_nonzero])
        .map_err(|_| CanonicalJsonError::InvalidValue)?;
    let adjusted_exponent = explicit_exponent + decimal_position - first_nonzero as i32 - 1;

    if !(-4..16).contains(&adjusted_exponent) {
        let mut coefficient = significant[..1].to_string();
        if significant.len() > 1 {
            coefficient.push('.');
            coefficient.push_str(&significant[1..]);
        }
        let exponent_sign = if adjusted_exponent < 0 { '-' } else { '+' };
        return Ok(format!(
            "{sign}{coefficient}e{exponent_sign}{:02}",
            adjusted_exponent.unsigned_abs()
        ));
    }

    let decimal_position = adjusted_exponent + 1;
    if decimal_position <= 0 {
        return Ok(format!(
            "{sign}0.{}{significant}",
            "0".repeat((-decimal_position) as usize)
        ));
    }
    let decimal_position = decimal_position as usize;
    if decimal_position >= significant.len() {
        return Ok(format!(
            "{sign}{significant}{}.0",
            "0".repeat(decimal_position - significant.len())
        ));
    }
    Ok(format!(
        "{sign}{}.{}",
        &significant[..decimal_position],
        &significant[decimal_position..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matches_immutable_cpython_311_float_and_unicode_golden() {
        let value = json!({
            "unicode": "Café ⚓",
            "fixed_pos_low": 1e-4,
            "scientific_pos_low": 1e-5,
            "fixed_pos_high": 1e15,
            "scientific_pos_high": 1e16,
            "negative_zero": -0.0,
            "min_subnormal": f64::from_bits(1),
            "max_finite": f64::MAX
        });
        assert_eq!(
            String::from_utf8(canonical_json_bytes(&value, 4096).expect("canonical"))
                .expect("UTF-8"),
            r#"{"fixed_pos_high":1000000000000000.0,"fixed_pos_low":0.0001,"max_finite":1.7976931348623157e+308,"min_subnormal":5e-324,"negative_zero":-0.0,"scientific_pos_high":1e+16,"scientific_pos_low":1e-05,"unicode":"Café ⚓"}"#
        );
    }
}

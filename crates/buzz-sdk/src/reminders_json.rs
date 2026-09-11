pub(super) fn parse(text: &str) -> Result<serde_json::Value, serde_json::Error> {
    use serde::de::{DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
    use serde_json::Value;
    use std::collections::HashSet;
    use std::fmt;

    struct StrictValue;

    impl<'de> DeserializeSeed<'de> for StrictValue {
        type Value = Value;
        fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
            d.deserialize_any(StrictValue)
        }
    }

    impl<'de> Visitor<'de> for StrictValue {
        type Value = Value;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("any valid JSON value (objects must have unique keys)")
        }

        fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
            Ok(Value::Bool(v))
        }
        fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
            Ok(Value::Number(v.into()))
        }
        fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
            Ok(Value::Number(v.into()))
        }
        fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
            serde_json::Number::from_f64(v)
                .map(Value::Number)
                .ok_or_else(|| E::custom("non-finite float"))
        }
        fn visit_str<E>(self, v: &str) -> Result<Value, E> {
            Ok(Value::String(v.to_owned()))
        }
        fn visit_string<E>(self, v: String) -> Result<Value, E> {
            Ok(Value::String(v))
        }
        fn visit_unit<E>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_none<E>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
            d.deserialize_any(StrictValue)
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
            let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0));
            while let Some(v) = seq.next_element_seed(StrictValue)? {
                out.push(v);
            }
            Ok(Value::Array(out))
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
            use serde::de::Error;
            let mut seen: HashSet<String> = HashSet::new();
            let mut out = serde_json::Map::new();
            while let Some(k) = map.next_key::<String>()? {
                if !seen.insert(k.clone()) {
                    return Err(A::Error::custom(format!(
                        "duplicate object member name: {k}"
                    )));
                }
                let v = map.next_value_seed(StrictValue)?;
                out.insert(k, v);
            }
            Ok(Value::Object(out))
        }
    }

    let mut de = serde_json::Deserializer::from_str(text);
    let value = StrictValue.deserialize(&mut de)?;
    de.end()?;
    Ok(value)
}

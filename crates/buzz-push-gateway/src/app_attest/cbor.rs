//! Bounded CBOR receiver profile, built on minicbor's primitive decoder.
//!
//! Map equality follows RFC 8949 section 5.6.1, including compound keys. The
//! ordered representation is for duplicate detection only, never for signing.
use super::error::AppAttestError;
use minicbor::{data::Type, Decoder};
use std::collections::BTreeMap;

// Operational ceilings, not Apple protocol constants. Keep these separate from
// transport limits and report capacity failures distinctly.
pub(super) const MAX_BYTES: usize = 64 * 1024;
const MAX_ITEMS: usize = 4096;
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Value {
    Integer(i128),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Value>),
    Map(BTreeMap<Value, Value>),
    Tag(u64, Box<Value>),
    Simple(u8),
    Float(u64),
    Nan(u64),
}

impl Value {
    pub(super) fn map(&self) -> Result<&BTreeMap<Value, Value>, AppAttestError> {
        match self {
            Self::Map(v) => Ok(v),
            _ => Err(AppAttestError::InvalidCbor),
        }
    }
    pub(super) fn bytes(&self) -> Result<&[u8], AppAttestError> {
        match self {
            Self::Bytes(v) => Ok(v),
            _ => Err(AppAttestError::InvalidCbor),
        }
    }
    pub(super) fn field(&self, key: &str) -> Result<&Value, AppAttestError> {
        self.map()?
            .get(&Value::Text(key.into()))
            .ok_or(AppAttestError::InvalidCbor)
    }
    pub(super) fn text_keys(&self) -> Result<(), AppAttestError> {
        if self.map()?.keys().all(|k| matches!(k, Self::Text(_))) {
            Ok(())
        } else {
            Err(AppAttestError::InvalidCbor)
        }
    }
}

pub(super) fn complete(bytes: &[u8]) -> Result<Value, AppAttestError> {
    let (value, consumed) = prefix(bytes)?;
    if consumed != bytes.len() {
        return Err(AppAttestError::InvalidCbor);
    }
    Ok(value)
}

pub(super) fn prefix(bytes: &[u8]) -> Result<(Value, usize), AppAttestError> {
    if bytes.len() > MAX_BYTES {
        return Err(AppAttestError::ResourceLimit);
    }
    let mut reader = Reader {
        decoder: Decoder::new(bytes),
        items: 0,
        copied: 0,
    };
    let value = reader.value(0)?;
    Ok((value, reader.decoder.position()))
}

struct Reader<'a> {
    decoder: Decoder<'a>,
    items: usize,
    copied: usize,
}
impl Reader<'_> {
    fn item(&mut self) -> Result<(), AppAttestError> {
        self.items += 1;
        if self.items > MAX_ITEMS {
            return Err(AppAttestError::ResourceLimit);
        }
        Ok(())
    }
    fn copy(&mut self, size: usize) -> Result<(), AppAttestError> {
        self.copied = self
            .copied
            .checked_add(size)
            .ok_or(AppAttestError::ResourceLimit)?;
        if self.copied > MAX_BYTES {
            return Err(AppAttestError::ResourceLimit);
        }
        Ok(())
    }
    fn end(&mut self, length: Option<u64>, seen: u64) -> Result<bool, AppAttestError> {
        if let Some(length) = length {
            return Ok(seen == length);
        }
        if self.decoder.datatype()? == Type::Break {
            self.decoder.skip()?;
            return Ok(true);
        }
        Ok(false)
    }
    fn value(&mut self, depth: usize) -> Result<Value, AppAttestError> {
        if depth > MAX_DEPTH {
            return Err(AppAttestError::ResourceLimit);
        }
        self.item()?;
        match self.decoder.datatype()? {
            Type::U8
            | Type::U16
            | Type::U32
            | Type::U64
            | Type::I8
            | Type::I16
            | Type::I32
            | Type::I64
            | Type::Int => Ok(Value::Integer(self.decoder.int()?.into())),
            Type::Bytes | Type::BytesIndef => {
                let mut out = Vec::new();
                // Each chunk is at most the bounded input size. No allocation
                // is based solely on an untrusted declared length.
                let chunks = self.decoder.bytes_iter()?;
                for chunk in chunks {
                    let chunk = chunk?;
                    if out.len() + chunk.len() > MAX_BYTES {
                        return Err(AppAttestError::ResourceLimit);
                    }
                    out.extend_from_slice(chunk);
                }
                self.copy(out.len())?;
                Ok(Value::Bytes(out))
            }
            Type::String | Type::StringIndef => {
                let mut out = String::new();
                for chunk in self.decoder.str_iter()? {
                    let chunk = chunk?;
                    if out.len() + chunk.len() > MAX_BYTES {
                        return Err(AppAttestError::ResourceLimit);
                    }
                    out.push_str(chunk);
                }
                self.copy(out.len())?;
                Ok(Value::Text(out))
            }
            Type::Array | Type::ArrayIndef => {
                let length = self.decoder.array()?;
                let mut out = Vec::new();
                while !self.end(length, out.len() as u64)? {
                    out.push(self.value(depth + 1)?);
                }
                Ok(Value::Array(out))
            }
            Type::Map | Type::MapIndef => {
                let length = self.decoder.map()?;
                let mut out = BTreeMap::new();
                while !self.end(length, out.len() as u64)? {
                    let key = self.value(depth + 1)?;
                    let value = self.value(depth + 1)?;
                    if out.insert(key, value).is_some() {
                        return Err(AppAttestError::InvalidCbor);
                    }
                }
                Ok(Value::Map(out))
            }
            Type::Tag => {
                let tag = self.decoder.tag()?.as_u64();
                Ok(Value::Tag(tag, Box::new(self.value(depth + 1)?)))
            }
            Type::Bool => Ok(Value::Simple(if self.decoder.bool()? { 21 } else { 20 })),
            Type::Null => {
                self.decoder.null()?;
                Ok(Value::Simple(22))
            }
            Type::Undefined => {
                self.decoder.undefined()?;
                Ok(Value::Simple(23))
            }
            Type::Simple => {
                let position = self.decoder.position();
                let value = self.decoder.simple()?;
                // RFC 8949 3.3: f8 followed by a value below 32 is not well formed.
                if self.decoder.position() - position == 2 && value < 32 {
                    return Err(AppAttestError::InvalidCbor);
                }
                Ok(Value::Simple(value))
            }
            Type::F16 | Type::F32 | Type::F64 => {
                let position = self.decoder.position();
                let number = self.decoder.f64()?;
                if number.is_nan() {
                    // Preserve signaling/payload bits before a floating-point
                    // conversion can quiet a NaN. The sign is not significand.
                    let raw = &self.decoder.input()[position + 1..self.decoder.position()];
                    let bits = raw.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b));
                    let width = match raw.len() {
                        2 => 10,
                        4 => 23,
                        8 => 52,
                        _ => return Err(AppAttestError::InvalidCbor),
                    };
                    Ok(Value::Nan((bits & ((1u64 << width) - 1)) << (64 - width)))
                } else {
                    Ok(Value::Float(if number == 0.0 {
                        0
                    } else {
                        number.to_bits()
                    }))
                }
            }
            _ => Err(AppAttestError::InvalidCbor),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_keys_use_generic_cbor_equivalence() {
        for bytes in [
            &b"\xa2\x01\x00\x18\x01\x01"[..],
            &b"\xa2\x81\x01\x00\x81\x18\x01\x01"[..],
            &b"\xa2\xa2\x01\x02\x03\x04\x00\xa2\x03\x04\x01\x02\x01"[..],
            &b"\xa2\xf9\x00\x00\x00\xf9\x80\x00\x01"[..],
            &b"\xa2\xf9\x7e\x00\x00\xfa\x7f\xc0\x00\x00\x01"[..],
            &b"\xa2\x61x\x00\x7f\x61x\xff\x01"[..],
        ] {
            assert!(complete(bytes).is_err(), "{bytes:?}");
        }
        assert!(complete(b"\xa2\x01\x00\xf9\x3c\x00\x01").is_ok());
    }
    #[test]
    fn bounded_opaque_values_and_nonpreferred_encodings() {
        assert!(complete(b"\xbf\x61x\x9f\xc1\xf8\xff\x18\x01\xff\xff").is_ok());
        for bytes in [
            &b"\xff"[..],
            &b"\xa0\x00"[..],
            &b"\xf8\x01"[..],
            &b"\x61\xff"[..],
            &b"\xbf\x01\xff"[..],
        ] {
            assert!(complete(bytes).is_err());
        }
        let mut deep = vec![0x81; MAX_DEPTH + 2];
        deep.push(0);
        assert_eq!(complete(&deep), Err(AppAttestError::ResourceLimit));
    }

    #[test]
    fn capacity_boundaries_and_untrusted_lengths() {
        let mut at_depth = vec![0x81; MAX_DEPTH];
        at_depth.push(0);
        assert!(complete(&at_depth).is_ok());
        let mut at_items = vec![0x9f];
        at_items.extend(std::iter::repeat_n(0, MAX_ITEMS - 1));
        at_items.push(0xff);
        assert!(complete(&at_items).is_ok());
        at_items.insert(1, 0);
        assert_eq!(complete(&at_items), Err(AppAttestError::ResourceLimit));
        let mut at_bytes = vec![0x59, 0xff, 0xfd];
        at_bytes.resize(MAX_BYTES, 0);
        assert!(complete(&at_bytes).is_ok());
        at_bytes.push(0);
        assert_eq!(complete(&at_bytes), Err(AppAttestError::ResourceLimit));
        for prefix in [0x5b, 0x7b, 0x9b, 0xbb] {
            let mut huge_length = vec![prefix];
            huge_length.extend_from_slice(&u64::MAX.to_be_bytes());
            assert!(complete(&huge_length).is_err());
        }
    }
    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(512))]
        #[test]
        fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..=MAX_BYTES)) {
            let _ = complete(&bytes);
        }
    }

    #[test]
    #[ignore = "manual resource characterization, not a timing-sensitive CI gate"]
    fn characterize_capacity_work() {
        use std::{hint::black_box, time::Instant};
        let mut at_items = vec![0x9f];
        at_items.extend(std::iter::repeat_n(0, MAX_ITEMS - 1));
        at_items.push(0xff);
        let mut at_bytes = vec![0x59, 0xff, 0xfd];
        at_bytes.resize(MAX_BYTES, 0);
        let mut at_depth = vec![0x81; MAX_DEPTH];
        at_depth.push(0);
        let mut map = minicbor::Encoder::new(Vec::new());
        map.map(((MAX_ITEMS - 1) / 2) as u64).unwrap();
        for index in 0..(MAX_ITEMS - 1) / 2 {
            map.str(&format!("shared-prefix-{index:08}"))
                .unwrap()
                .u8(0)
                .unwrap();
        }
        for (name, bytes) in [
            ("map", map.into_writer()),
            ("items", at_items),
            ("bytes", at_bytes),
            ("depth", at_depth),
        ] {
            let start = Instant::now();
            for _ in 0..1000 {
                black_box(complete(black_box(&bytes)).unwrap());
            }
            eprintln!(
                "{name}: {} input bytes, {} ns/parse (1000 iterations)",
                bytes.len(),
                start.elapsed().as_nanos() / 1000
            );
        }
    }
}

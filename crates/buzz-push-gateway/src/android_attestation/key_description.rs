//! Parser for the Android Key Attestation extension (`KeyDescription`).
//!
//! Only the fields the gateway enforces are decoded. The software-enforced
//! list is attacker-controlled on a compromised device, so the parser exposes
//! the hardware-enforced properties and the application id separately and the
//! caller never has to choose between them.

/// OID of the Key Attestation extension, 1.3.6.1.4.1.11129.2.1.17.
pub(super) const KEY_DESCRIPTION_OID: &str = "1.3.6.1.4.1.11129.2.1.17";

/// Android `SecurityLevel`.
pub(super) const TRUSTED_ENVIRONMENT: u64 = 1;
/// Android `SecurityLevel`.
pub(super) const STRONG_BOX: u64 = 2;

const TAG_PURPOSE: u32 = 1;
const TAG_ALGORITHM: u32 = 2;
const TAG_EC_CURVE: u32 = 10;
const TAG_ORIGIN: u32 = 702;
const TAG_ROOT_OF_TRUST: u32 = 704;
const TAG_APPLICATION_ID: u32 = 709;

const UNIVERSAL_INTEGER: u8 = 0x02;
const UNIVERSAL_OCTET_STRING: u8 = 0x04;
const UNIVERSAL_BOOLEAN: u8 = 0x01;
const UNIVERSAL_ENUMERATED: u8 = 0x0a;
const UNIVERSAL_SEQUENCE: u8 = 0x30;
const UNIVERSAL_SET: u8 = 0x31;

/// One decoded DER element.
#[derive(Clone, Copy)]
struct Tlv<'a> {
    /// First identifier octet for universal tags, otherwise the class and
    /// constructed bits with the tag number in `number`.
    class_byte: u8,
    number: u32,
    content: &'a [u8],
}

/// Read one definite-length DER element and return it with the remainder.
fn read(input: &[u8]) -> Option<(Tlv<'_>, &[u8])> {
    let (&first, rest) = input.split_first()?;
    let mut number = u32::from(first & 0x1f);
    let mut rest = rest;
    if number == 0x1f {
        // High tag number form: base-128, minimal, at most 3 octets.
        number = 0;
        for index in 0..3 {
            let (&octet, tail) = rest.split_first()?;
            rest = tail;
            if index == 0 && octet == 0x80 {
                return None;
            }
            number = (number << 7) | u32::from(octet & 0x7f);
            if octet & 0x80 == 0 {
                break;
            }
            if index == 2 {
                return None;
            }
        }
        if number < 0x1f {
            return None;
        }
    }
    let (&length_octet, rest) = rest.split_first()?;
    let (length, rest) = if length_octet < 0x80 {
        (usize::from(length_octet), rest)
    } else {
        let count = usize::from(length_octet & 0x7f);
        // Indefinite lengths and lengths wider than 3 octets are not DER we accept.
        if count == 0 || count > 3 || rest.len() < count || rest[0] == 0 {
            return None;
        }
        let length = rest[..count]
            .iter()
            .fold(0usize, |acc, octet| (acc << 8) | usize::from(*octet));
        if length < 0x80 {
            return None;
        }
        (length, &rest[count..])
    };
    if rest.len() < length {
        return None;
    }
    let (content, tail) = rest.split_at(length);
    Some((
        Tlv {
            class_byte: first & 0xe0,
            number,
            content,
        },
        tail,
    ))
}

/// Read one element that must be universal and carry `tag` as its full
/// identifier octet, such as `0x30` for SEQUENCE.
fn read_universal(input: &[u8], tag: u8) -> Option<(&[u8], &[u8])> {
    let (&first, _) = input.split_first()?;
    if first != tag {
        return None;
    }
    let (tlv, rest) = read(input)?;
    Some((tlv.content, rest))
}

fn small_unsigned(content: &[u8]) -> Option<u64> {
    // Non-negative, minimal, and small enough for every field we read.
    match content {
        [] => None,
        [first, ..] if *first & 0x80 != 0 || content.len() > 8 => None,
        [0, second, ..] if *second & 0x80 == 0 => None,
        _ => Some(
            content
                .iter()
                .fold(0u64, |acc, b| (acc << 8) | u64::from(*b)),
        ),
    }
}

fn read_integer(input: &[u8], tag: u8) -> Option<(u64, &[u8])> {
    let (content, rest) = read_universal(input, tag)?;
    Some((small_unsigned(content)?, rest))
}

/// Boot state reported in the hardware-enforced `RootOfTrust`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VerifiedBootState {
    Verified,
    SelfSigned,
    Unverified,
    Failed,
}

/// The hardware-enforced `RootOfTrust`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RootOfTrust {
    pub verified_boot_key: Vec<u8>,
    pub device_locked: bool,
    pub verified_boot_state: VerifiedBootState,
}

/// The fields of the hardware-enforced authorization list the gateway checks.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct HardwareEnforced {
    pub purposes: Option<Vec<u64>>,
    pub algorithm: Option<u64>,
    pub ec_curve: Option<u64>,
    pub origin: Option<u64>,
    pub root_of_trust: Option<RootOfTrust>,
}

/// The package and signing digests the operating system recorded for the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ApplicationId {
    pub package_names: Vec<Vec<u8>>,
    pub signature_digests: Vec<Vec<u8>>,
}

/// A decoded `KeyDescription`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct KeyDescription {
    pub attestation_security_level: u64,
    pub keymaster_security_level: u64,
    pub challenge: Vec<u8>,
    pub hardware: HardwareEnforced,
    /// Read from the software-enforced list: the operating system fills it in.
    pub application_id: Option<ApplicationId>,
}

/// Decode a `KeyDescription` from the extension's value. Returns `None` for
/// anything malformed, including trailing bytes and duplicate fields.
pub(super) fn parse(extension_value: &[u8]) -> Option<KeyDescription> {
    let (body, rest) = read_universal(extension_value, UNIVERSAL_SEQUENCE)?;
    if !rest.is_empty() {
        return None;
    }
    let (_version, body) = read_integer(body, UNIVERSAL_INTEGER)?;
    let (attestation_security_level, body) = read_integer(body, UNIVERSAL_ENUMERATED)?;
    let (_keymaster_version, body) = read_integer(body, UNIVERSAL_INTEGER)?;
    let (keymaster_security_level, body) = read_integer(body, UNIVERSAL_ENUMERATED)?;
    let (challenge, body) = read_universal(body, UNIVERSAL_OCTET_STRING)?;
    let (_unique_id, body) = read_universal(body, UNIVERSAL_OCTET_STRING)?;
    let (software, body) = read_universal(body, UNIVERSAL_SEQUENCE)?;
    let (hardware, body) = read_universal(body, UNIVERSAL_SEQUENCE)?;
    if !body.is_empty() {
        return None;
    }
    Some(KeyDescription {
        attestation_security_level,
        keymaster_security_level,
        challenge: challenge.to_vec(),
        hardware: parse_hardware(hardware)?,
        application_id: parse_software_application_id(software)?,
    })
}

/// Iterate an authorization list's explicitly tagged fields, failing on a
/// malformed element or a repeated tag.
fn fields(list: &[u8]) -> Option<Vec<(u32, &[u8])>> {
    let mut out: Vec<(u32, &[u8])> = Vec::new();
    let mut rest = list;
    while !rest.is_empty() {
        let (tlv, tail) = read(rest)?;
        // Context-specific, constructed (explicit tagging).
        if tlv.class_byte != 0xa0 || out.iter().any(|(number, _)| *number == tlv.number) {
            return None;
        }
        out.push((tlv.number, tlv.content));
        rest = tail;
    }
    Some(out)
}

fn explicit_integer(content: &[u8]) -> Option<u64> {
    let (value, rest) = read_integer(content, UNIVERSAL_INTEGER)?;
    rest.is_empty().then_some(value)
}

fn parse_hardware(list: &[u8]) -> Option<HardwareEnforced> {
    let mut out = HardwareEnforced::default();
    for (number, content) in fields(list)? {
        match number {
            TAG_PURPOSE => {
                let (set, rest) = read_universal(content, UNIVERSAL_SET)?;
                if !rest.is_empty() {
                    return None;
                }
                let mut purposes = Vec::new();
                let mut items = set;
                while !items.is_empty() {
                    let (value, tail) = read_integer(items, UNIVERSAL_INTEGER)?;
                    purposes.push(value);
                    items = tail;
                }
                out.purposes = Some(purposes);
            }
            TAG_ALGORITHM => out.algorithm = Some(explicit_integer(content)?),
            TAG_EC_CURVE => out.ec_curve = Some(explicit_integer(content)?),
            TAG_ORIGIN => out.origin = Some(explicit_integer(content)?),
            TAG_ROOT_OF_TRUST => out.root_of_trust = Some(parse_root_of_trust(content)?),
            _ => {}
        }
    }
    Some(out)
}

fn parse_root_of_trust(content: &[u8]) -> Option<RootOfTrust> {
    let (body, rest) = read_universal(content, UNIVERSAL_SEQUENCE)?;
    if !rest.is_empty() {
        return None;
    }
    let (key, body) = read_universal(body, UNIVERSAL_OCTET_STRING)?;
    let (locked, body) = read_universal(body, UNIVERSAL_BOOLEAN)?;
    let device_locked = match locked {
        [0x00] => false,
        [0xff] => true,
        _ => return None,
    };
    let (state, _hash_and_extensions) = read_integer(body, UNIVERSAL_ENUMERATED)?;
    let verified_boot_state = match state {
        0 => VerifiedBootState::Verified,
        1 => VerifiedBootState::SelfSigned,
        2 => VerifiedBootState::Unverified,
        3 => VerifiedBootState::Failed,
        _ => return None,
    };
    Some(RootOfTrust {
        verified_boot_key: key.to_vec(),
        device_locked,
        verified_boot_state,
    })
}

/// `Some(None)` when the software list has no application id, `None` when it
/// is malformed.
fn parse_software_application_id(list: &[u8]) -> Option<Option<ApplicationId>> {
    let Some((_, content)) = fields(list)?
        .into_iter()
        .find(|(number, _)| *number == TAG_APPLICATION_ID)
    else {
        return Some(None);
    };
    let (octets, rest) = read_universal(content, UNIVERSAL_OCTET_STRING)?;
    if !rest.is_empty() {
        return None;
    }
    let (body, rest) = read_universal(octets, UNIVERSAL_SEQUENCE)?;
    if !rest.is_empty() {
        return None;
    }
    let (packages, body) = read_universal(body, UNIVERSAL_SET)?;
    let (digests, body) = read_universal(body, UNIVERSAL_SET)?;
    if !body.is_empty() {
        return None;
    }
    let mut package_names = Vec::new();
    let mut items = packages;
    while !items.is_empty() {
        let (info, tail) = read_universal(items, UNIVERSAL_SEQUENCE)?;
        let (name, info_rest) = read_universal(info, UNIVERSAL_OCTET_STRING)?;
        let (_version, info_rest) = read_integer(info_rest, UNIVERSAL_INTEGER)?;
        if !info_rest.is_empty() {
            return None;
        }
        package_names.push(name.to_vec());
        items = tail;
    }
    let mut signature_digests = Vec::new();
    let mut items = digests;
    while !items.is_empty() {
        let (digest, tail) = read_universal(items, UNIVERSAL_OCTET_STRING)?;
        signature_digests.push(digest.to_vec());
        items = tail;
    }
    Some(Some(ApplicationId {
        package_names,
        signature_digests,
    }))
}

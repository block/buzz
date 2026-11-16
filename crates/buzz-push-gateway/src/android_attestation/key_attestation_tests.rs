//! Key Attestation chains generated here, with a test root injected as the
//! configured root, run through the production verifier.
use super::*;
use crate::android_attestation::revocation::RevocationList;
use openssl::{
    asn1::{Asn1Object, Asn1OctetString, Asn1Time},
    bn::BigNum,
    ec::{EcGroup, EcKey},
    hash::MessageDigest,
    nid::Nid,
    pkey::{PKey, Private},
    x509::{
        extension::{BasicConstraints, KeyUsage},
        X509Builder, X509Extension, X509NameBuilder, X509,
    },
};
use std::collections::HashSet;

const NOW: i64 = 1_750_000_000;
const PACKAGE: &str = "com.example.buzz";
const DIGEST: [u8; 32] = [0xab; 32];
const BOOT_KEY: [u8; 32] = [0x11; 32];
const TRANSCRIPT: &[u8] = b"buzz.push.enroll.v1\n{\"challenge\":\"example\"}";

fn now() -> i64 {
    NOW
}

// DER building blocks.
fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match content.len() {
        n if n < 0x80 => out.push(n as u8),
        n if n < 0x100 => out.extend([0x81, n as u8]),
        n => out.extend([0x82, (n >> 8) as u8, n as u8]),
    }
    out.extend_from_slice(content);
    out
}
fn int(value: u64, tag: u8) -> Vec<u8> {
    let mut bytes: Vec<u8> = value
        .to_be_bytes()
        .into_iter()
        .skip_while(|b| *b == 0)
        .collect();
    if bytes.first().is_none_or(|b| b & 0x80 != 0) {
        bytes.insert(0, 0);
    }
    tlv(tag, &bytes)
}
fn octets(bytes: &[u8]) -> Vec<u8> {
    tlv(0x04, bytes)
}
/// An explicitly tagged, constructed context-specific field.
fn tagged(number: u32, content: &[u8]) -> Vec<u8> {
    if number < 31 {
        return tlv(0xa0 | number as u8, content);
    }
    let mut out = vec![0xbf, 0x80 | (number >> 7) as u8, (number & 0x7f) as u8];
    let length = tlv(0, content);
    out.extend_from_slice(&length[1..]);
    out
}

/// One attestation as the device's secure hardware and operating system
/// would write it. `Default` is acceptable to a default gateway.
#[derive(Clone)]
struct Spec {
    attestation_level: u64,
    keymaster_level: u64,
    /// Overrides the challenge, which is otherwise SHA-256 of the transcript.
    challenge: Option<Vec<u8>>,
    locked: bool,
    boot_state: u64,
    boot_key: Vec<u8>,
    hardware_root_of_trust: bool,
    /// A root of trust placed in the attacker-controlled software list.
    software_root_of_trust: Option<(bool, u64, Vec<u8>)>,
    hardware_key_properties: bool,
    software_key_properties: bool,
    purposes: Vec<u64>,
    curve: u64,
    packages: Vec<&'static str>,
    digests: Vec<[u8; 32]>,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            attestation_level: STRONG_BOX,
            keymaster_level: STRONG_BOX,
            challenge: None,
            locked: true,
            boot_state: 0,
            boot_key: vec![0x77; 32],
            hardware_root_of_trust: true,
            software_root_of_trust: None,
            hardware_key_properties: true,
            software_key_properties: false,
            purposes: vec![2],
            curve: 1,
            packages: vec![PACKAGE],
            digests: vec![DIGEST],
        }
    }
}

fn root_of_trust(locked: bool, state: u64, key: &[u8]) -> Vec<u8> {
    let body = [
        octets(key),
        tlv(0x01, &[if locked { 0xff } else { 0x00 }]),
        int(state, 0x0a),
        octets(&[0; 32]),
    ]
    .concat();
    tagged(704, &tlv(0x30, &body))
}

fn key_properties(spec: &Spec) -> Vec<u8> {
    let purposes: Vec<u8> = spec.purposes.iter().flat_map(|p| int(*p, 0x02)).collect();
    [
        tagged(1, &tlv(0x31, &purposes)),
        tagged(2, &int(3, 0x02)),
        tagged(10, &int(spec.curve, 0x02)),
        tagged(702, &int(0, 0x02)),
    ]
    .concat()
}

fn key_description(spec: &Spec, transcript: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let challenge = spec
        .challenge
        .clone()
        .unwrap_or_else(|| Sha256::digest(transcript).to_vec());
    let packages: Vec<u8> = spec
        .packages
        .iter()
        .flat_map(|name| tlv(0x30, &[octets(name.as_bytes()), int(1, 0x02)].concat()))
        .collect();
    let digests: Vec<u8> = spec.digests.iter().flat_map(|d| octets(d)).collect();
    let application_id = tlv(0x30, &[tlv(0x31, &packages), tlv(0x31, &digests)].concat());
    let mut software = Vec::new();
    if spec.software_key_properties {
        software.extend(key_properties(spec));
    }
    if let Some((locked, state, key)) = &spec.software_root_of_trust {
        software.extend(root_of_trust(*locked, *state, key));
    }
    software.extend(tagged(709, &octets(&application_id)));
    let mut hardware = Vec::new();
    if spec.hardware_key_properties {
        hardware.extend(key_properties(spec));
    }
    if spec.hardware_root_of_trust {
        hardware.extend(root_of_trust(spec.locked, spec.boot_state, &spec.boot_key));
    }
    tlv(
        0x30,
        &[
            int(300, 0x02),
            int(spec.attestation_level, 0x0a),
            int(300, 0x02),
            int(spec.keymaster_level, 0x0a),
            octets(&challenge),
            octets(&[]),
            tlv(0x30, &software),
            tlv(0x30, &hardware),
        ]
        .concat(),
    )
}

/// How the certificate chain around the attestation is built.
#[derive(Clone, Default)]
struct Pki {
    /// The intermediate expired before `NOW`.
    expired_intermediate: bool,
    /// The intermediate carries the Remote Key Provisioning extension.
    provisioned: bool,
    /// The chain ends at a root the gateway does not trust.
    rogue_root: bool,
    /// The leaf is signed by a key other than the intermediate's.
    forged_leaf_signature: bool,
    /// A copy of the attestation extension on the intermediate.
    extension_on_intermediate: bool,
    leaf_p384: bool,
    /// The leaf certificate encodes its key as a compressed point.
    leaf_compressed: bool,
    /// The intermediate is CA:FALSE with digitalSignature only, as some
    /// factory attestation key certificates are.
    intermediate_not_ca: bool,
    /// A CA:FALSE, digitalSignature certificate between the root and the
    /// intermediate issues the intermediate.
    non_ca_issues_intermediate: bool,
}

fn ec_key(nid: Nid) -> PKey<Private> {
    PKey::from_ec_key(EcKey::generate(&EcGroup::from_curve_name(nid).unwrap()).unwrap()).unwrap()
}

fn name(common_name: &str) -> openssl::x509::X509Name {
    let mut builder = X509NameBuilder::new().unwrap();
    builder.append_entry_by_text("CN", common_name).unwrap();
    builder.build()
}

struct Issuer<'a> {
    name: &'a openssl::x509::X509NameRef,
    key: &'a PKey<Private>,
}

fn certificate(
    serial: u32,
    subject: &str,
    key: &PKey<Private>,
    issuer: Option<Issuer<'_>>,
    ca: bool,
    not_after: &str,
    extensions: Vec<X509Extension>,
) -> X509 {
    let subject_name = name(subject);
    let mut builder = X509Builder::new().unwrap();
    builder.set_version(2).unwrap();
    builder
        .set_serial_number(&BigNum::from_u32(serial).unwrap().to_asn1_integer().unwrap())
        .unwrap();
    builder.set_subject_name(&subject_name).unwrap();
    builder
        .set_issuer_name(issuer.as_ref().map_or(&*subject_name, |i| i.name))
        .unwrap();
    builder.set_pubkey(key).unwrap();
    builder
        .set_not_before(&Asn1Time::from_str("20200101000000Z").unwrap())
        .unwrap();
    builder
        .set_not_after(&Asn1Time::from_str(not_after).unwrap())
        .unwrap();
    if ca {
        builder
            .append_extension(BasicConstraints::new().critical().ca().build().unwrap())
            .unwrap();
        builder
            .append_extension(KeyUsage::new().critical().key_cert_sign().build().unwrap())
            .unwrap();
    }
    for extension in extensions {
        builder.append_extension(extension).unwrap();
    }
    let signer = issuer.as_ref().map_or(key, |i| i.key);
    let digest = match signer.ec_key().unwrap().group().curve_name() {
        Some(Nid::SECP384R1) => MessageDigest::sha384(),
        _ => MessageDigest::sha256(),
    };
    builder.sign(signer, digest).unwrap();
    builder.build()
}

fn custom_extension(oid: &str, der: &[u8]) -> X509Extension {
    X509Extension::new_from_der(
        &Asn1Object::from_str(oid).unwrap(),
        false,
        &Asn1OctetString::new_from_bytes(der).unwrap(),
    )
    .unwrap()
}

/// A generated chain: DER certificates leaf first, and the trusted root.
struct Generated {
    chain: Vec<Vec<u8>>,
    root_pem: String,
}

const ROOT_SERIAL: u32 = 0x10;
const INTERMEDIATE_SERIAL: u32 = 0x20;
const LEAF_SERIAL: u32 = 0x30;
const UPPER_SERIAL: u32 = 0x40;

/// Basic constraints CA:FALSE and key usage digitalSignature only, the shape
/// of some factory attestation key certificates.
fn not_a_ca() -> Vec<X509Extension> {
    vec![
        BasicConstraints::new().critical().build().unwrap(),
        KeyUsage::new()
            .critical()
            .digital_signature()
            .build()
            .unwrap(),
    ]
}

fn generate(spec: &Spec, pki: &Pki, transcript: &[u8]) -> Generated {
    let trusted_root_key = ec_key(Nid::SECP384R1);
    let trusted_root = certificate(
        ROOT_SERIAL,
        "Example Attestation Root",
        &trusted_root_key,
        None,
        true,
        "20400101000000Z",
        vec![],
    );
    let (signing_root_key, signing_root) = if pki.rogue_root {
        let key = ec_key(Nid::SECP384R1);
        let cert = certificate(
            ROOT_SERIAL,
            "Example Attestation Root",
            &key,
            None,
            true,
            "20400101000000Z",
            vec![],
        );
        (key, cert)
    } else {
        (trusted_root_key.clone(), trusted_root.clone())
    };
    let intermediate_key = ec_key(Nid::X9_62_PRIME256V1);
    let mut intermediate_extensions = Vec::new();
    if pki.provisioned {
        intermediate_extensions.push(custom_extension(PROVISIONING_INFO_OID, &octets(&[1])));
    }
    if pki.intermediate_not_ca {
        intermediate_extensions.extend(not_a_ca());
    }
    let upper_key = ec_key(Nid::X9_62_PRIME256V1);
    let upper = pki.non_ca_issues_intermediate.then(|| {
        certificate(
            UPPER_SERIAL,
            "Example Attestation Key Issuer",
            &upper_key,
            Some(Issuer {
                name: signing_root.subject_name(),
                key: &signing_root_key,
            }),
            false,
            "20400101000000Z",
            not_a_ca(),
        )
    });
    let (intermediate_issuer_name, intermediate_issuer_key) = match &upper {
        Some(upper) => (upper.subject_name(), &upper_key),
        None => (signing_root.subject_name(), &signing_root_key),
    };
    let description = key_description(spec, transcript);
    if pki.extension_on_intermediate {
        intermediate_extensions.push(custom_extension(KEY_DESCRIPTION_OID, &description));
    }
    let intermediate = certificate(
        INTERMEDIATE_SERIAL,
        "Example Attestation Intermediate",
        &intermediate_key,
        Some(Issuer {
            name: intermediate_issuer_name,
            key: intermediate_issuer_key,
        }),
        !pki.intermediate_not_ca,
        if pki.expired_intermediate {
            "20210101000000Z"
        } else {
            "20400101000000Z"
        },
        intermediate_extensions,
    );
    let leaf_key = ec_key(if pki.leaf_p384 {
        Nid::SECP384R1
    } else {
        Nid::X9_62_PRIME256V1
    });
    let leaf_key = if pki.leaf_compressed {
        compressed(&leaf_key)
    } else {
        leaf_key
    };
    let other_signer = ec_key(Nid::X9_62_PRIME256V1);
    let leaf = certificate(
        LEAF_SERIAL,
        "Example Attested Key",
        &leaf_key,
        Some(Issuer {
            name: intermediate.subject_name(),
            key: if pki.forged_leaf_signature {
                &other_signer
            } else {
                &intermediate_key
            },
        }),
        false,
        "20400101000000Z",
        vec![custom_extension(KEY_DESCRIPTION_OID, &description)],
    );
    Generated {
        chain: [
            Some(&leaf),
            Some(&intermediate),
            upper.as_ref(),
            Some(&signing_root),
        ]
        .into_iter()
        .flatten()
        .map(|cert| cert.to_der().unwrap())
        .collect(),
        root_pem: String::from_utf8(trusted_root.to_pem().unwrap()).unwrap(),
    }
}

/// The same P-256 key, parsed from an ECPrivateKey that encodes its public
/// key as a compressed point, so a certificate built from it keeps that form.
fn compressed(key: &PKey<Private>) -> PKey<Private> {
    let ec = key.ec_key().unwrap();
    let mut context = openssl::bn::BigNumContext::new().unwrap();
    let point = ec
        .public_key()
        .to_bytes(
            ec.group(),
            openssl::ec::PointConversionForm::COMPRESSED,
            &mut context,
        )
        .unwrap();
    // SEQUENCE { 1, OCTET STRING d, [0] prime256v1, [1] BIT STRING point }
    let mut der = hex::decode("30570201010420").unwrap();
    der.extend_from_slice(&ec.private_key().to_vec_padded(32).unwrap());
    der.extend_from_slice(&hex::decode("a00a06082a8648ce3d030107a124032200").unwrap());
    der.extend_from_slice(&point);
    PKey::from_ec_key(EcKey::private_key_from_der(&der).unwrap()).unwrap()
}

fn fresh_list(revoked: &[u32]) -> Arc<RevocationList> {
    let list = Arc::new(RevocationList::new(7 * 86_400));
    list.replace(
        revoked
            .iter()
            .map(|serial| serial_key(&serial.to_be_bytes()))
            .collect::<HashSet<_>>(),
        NOW,
    );
    list
}

fn policy() -> KeyAttestationPolicy {
    KeyAttestationPolicy {
        package_name: PACKAGE.into(),
        signing_cert_digests: vec![[0xee; 32], DIGEST],
        boot_key_digests: vec![BOOT_KEY],
    }
}

fn verifier(
    generated: &Generated,
    policy: KeyAttestationPolicy,
    list: Arc<RevocationList>,
) -> KeyAttestationVerifier {
    KeyAttestationVerifier::new(&generated.root_pem, policy, list, now).unwrap()
}

fn verify_with(
    spec: &Spec,
    pki: &Pki,
    policy: KeyAttestationPolicy,
    list: Arc<RevocationList>,
) -> Result<VerifiedAndroidKey, &'static str> {
    let generated = generate(spec, pki, TRANSCRIPT);
    verifier(&generated, policy, list).check(&generated.chain, TRANSCRIPT)
}

fn verify(spec: &Spec, pki: &Pki) -> Result<VerifiedAndroidKey, &'static str> {
    verify_with(spec, pki, policy(), fresh_list(&[]))
}

#[test]
fn accepted_attestations_prove_the_leaf_key() {
    let self_signed = |key: &[u8]| Spec {
        boot_state: 1,
        boot_key: key.to_vec(),
        ..Spec::default()
    };
    for (row, spec, pki) in [
        ("verified, strongbox", Spec::default(), Pki::default()),
        (
            "verified, trusted environment",
            Spec {
                attestation_level: TRUSTED_ENVIRONMENT,
                keymaster_level: TRUSTED_ENVIRONMENT,
                ..Spec::default()
            },
            Pki::default(),
        ),
        (
            "verified with an unlisted boot key",
            Spec {
                boot_key: vec![0x99; 32],
                ..Spec::default()
            },
            Pki::default(),
        ),
        (
            "self-signed with a listed key",
            self_signed(&BOOT_KEY),
            Pki::default(),
        ),
        (
            "second of several signing digests",
            Spec {
                digests: vec![[0x01; 32], DIGEST],
                ..Spec::default()
            },
            Pki::default(),
        ),
        (
            "expired factory-provisioned intermediate",
            Spec::default(),
            Pki {
                expired_intermediate: true,
                ..Pki::default()
            },
        ),
        (
            "current remote-provisioned intermediate",
            Spec::default(),
            Pki {
                provisioned: true,
                ..Pki::default()
            },
        ),
        (
            "factory attestation key certificate that is not a CA",
            Spec::default(),
            Pki {
                intermediate_not_ca: true,
                ..Pki::default()
            },
        ),
        (
            "software list claims a different boot state",
            Spec {
                software_root_of_trust: Some((false, 3, vec![0x55; 32])),
                ..Spec::default()
            },
            Pki::default(),
        ),
    ] {
        let proven = verify(&spec, &pki).unwrap_or_else(|reason| panic!("{row}: {reason}"));
        assert_eq!(proven.public_key.len(), 65, "{row}");
        assert_eq!(proven.public_key[0], 4, "{row}");
    }
}

/// The proven key is the leaf's key as an uncompressed point, however the
/// certificate encodes it, so one key always has one key id.
#[test]
fn the_proven_key_is_the_leaf_key() {
    for (compressed, spki_len) in [(false, 91), (true, 59)] {
        let pki = Pki {
            leaf_compressed: compressed,
            ..Pki::default()
        };
        let generated = generate(&Spec::default(), &pki, TRANSCRIPT);
        let proven = verifier(&generated, policy(), fresh_list(&[]))
            .check(&generated.chain, TRANSCRIPT)
            .unwrap();
        let leaf = X509::from_der(&generated.chain[0]).unwrap();
        let public = leaf.public_key().unwrap();
        assert_eq!(public.public_key_to_der().unwrap().len(), spki_len);
        let ec = public.ec_key().unwrap();
        let mut context = openssl::bn::BigNumContext::new().unwrap();
        let expected = ec
            .public_key()
            .to_bytes(
                ec.group(),
                openssl::ec::PointConversionForm::UNCOMPRESSED,
                &mut context,
            )
            .unwrap();
        assert_eq!(proven.public_key, expected, "compressed: {compressed}");
    }
}

#[test]
fn rejected_attestations() {
    let weak_boot = |locked: bool, state: u64, key: &[u8]| Spec {
        locked,
        boot_state: state,
        boot_key: key.to_vec(),
        ..Spec::default()
    };
    let rows: Vec<(&str, Spec, Pki, &str)> = vec![
        (
            "unlocked bootloader",
            weak_boot(false, 0, &[0x77; 32]),
            Pki::default(),
            "bootloader_unlocked",
        ),
        (
            "unlocked bootloader with a listed self-signed key",
            weak_boot(false, 1, &BOOT_KEY),
            Pki::default(),
            "bootloader_unlocked",
        ),
        (
            "unverified",
            weak_boot(true, 2, &BOOT_KEY),
            Pki::default(),
            "boot_state_rejected",
        ),
        (
            "failed",
            weak_boot(true, 3, &BOOT_KEY),
            Pki::default(),
            "boot_state_rejected",
        ),
        (
            "self-signed with an unlisted key",
            weak_boot(true, 1, &[0x42; 32]),
            Pki::default(),
            "boot_key_not_accepted",
        ),
        (
            "boot key only in the software list",
            Spec {
                hardware_root_of_trust: false,
                software_root_of_trust: Some((true, 0, BOOT_KEY.to_vec())),
                ..Spec::default()
            },
            Pki::default(),
            "no_root_of_trust",
        ),
        (
            "listed boot key only in the software list, hardware self-signed with another",
            Spec {
                boot_state: 1,
                boot_key: vec![0x42; 32],
                software_root_of_trust: Some((true, 1, BOOT_KEY.to_vec())),
                ..Spec::default()
            },
            Pki::default(),
            "boot_key_not_accepted",
        ),
        (
            "key properties only in the software list",
            Spec {
                hardware_key_properties: false,
                software_key_properties: true,
                ..Spec::default()
            },
            Pki::default(),
            "key_properties",
        ),
        (
            "key usable for more than signing",
            Spec {
                purposes: vec![2, 3],
                ..Spec::default()
            },
            Pki::default(),
            "key_properties",
        ),
        (
            "not a P-256 attested key",
            Spec {
                curve: 3,
                ..Spec::default()
            },
            Pki::default(),
            "key_properties",
        ),
        (
            "wrong package",
            Spec {
                packages: vec!["com.example.other"],
                ..Spec::default()
            },
            Pki::default(),
            "package_mismatch",
        ),
        (
            "two packages",
            Spec {
                packages: vec![PACKAGE, "com.example.other"],
                ..Spec::default()
            },
            Pki::default(),
            "package_mismatch",
        ),
        (
            "wrong signing digest",
            Spec {
                digests: vec![[0x01; 32]],
                ..Spec::default()
            },
            Pki::default(),
            "signing_digest_mismatch",
        ),
        (
            "wrong challenge",
            Spec {
                challenge: Some(vec![0; 32]),
                ..Spec::default()
            },
            Pki::default(),
            "challenge_mismatch",
        ),
        (
            "software-backed key",
            Spec {
                attestation_level: 0,
                keymaster_level: 0,
                ..Spec::default()
            },
            Pki::default(),
            "not_hardware_backed",
        ),
        (
            "software-backed KeyMint behind a hardware attestation",
            Spec {
                keymaster_level: 0,
                ..Spec::default()
            },
            Pki::default(),
            "not_hardware_backed",
        ),
        (
            "unknown root",
            Spec::default(),
            Pki {
                rogue_root: true,
                ..Pki::default()
            },
            "untrusted_root",
        ),
        (
            "leaf not signed by its issuer",
            Spec::default(),
            Pki {
                forged_leaf_signature: true,
                ..Pki::default()
            },
            "bad_signature",
        ),
        (
            "expired remote-provisioned intermediate",
            Spec::default(),
            Pki {
                provisioned: true,
                expired_intermediate: true,
                ..Pki::default()
            },
            "certificate_expired",
        ),
        (
            "attestation extension repeated above the leaf",
            Spec::default(),
            Pki {
                extension_on_intermediate: true,
                ..Pki::default()
            },
            "attestation_extension_misplaced",
        ),
        (
            "issuer of the attestation key certificate is not a CA",
            Spec::default(),
            Pki {
                non_ca_issues_intermediate: true,
                ..Pki::default()
            },
            "issuer_not_a_ca",
        ),
        (
            "leaf key is not P-256",
            Spec::default(),
            Pki {
                leaf_p384: true,
                ..Pki::default()
            },
            "leaf_key_not_p256",
        ),
    ];
    for (row, spec, pki, reason) in rows {
        assert_eq!(verify(&spec, &pki).err(), Some(reason), "{row}");
    }
}

#[test]
fn an_empty_boot_key_list_accepts_stock_android_only() {
    let stock_only = KeyAttestationPolicy {
        boot_key_digests: vec![],
        ..policy()
    };
    let self_signed = Spec {
        boot_state: 1,
        boot_key: BOOT_KEY.to_vec(),
        ..Spec::default()
    };
    assert_eq!(
        verify_with(
            &self_signed,
            &Pki::default(),
            stock_only.clone(),
            fresh_list(&[])
        )
        .err(),
        Some("boot_key_not_accepted")
    );
    assert!(verify_with(
        &Spec::default(),
        &Pki::default(),
        stock_only,
        fresh_list(&[])
    )
    .is_ok());
}

#[test]
fn listed_certificates_and_unusable_lists_are_rejected() {
    for (row, serial) in [
        ("leaf", LEAF_SERIAL),
        ("intermediate", INTERMEDIATE_SERIAL),
        ("root", ROOT_SERIAL),
    ] {
        assert_eq!(
            verify_with(
                &Spec::default(),
                &Pki::default(),
                policy(),
                fresh_list(&[serial])
            )
            .err(),
            Some("certificate_revoked"),
            "{row}"
        );
    }
    // A serial that is not in the chain does not matter.
    assert!(verify_with(
        &Spec::default(),
        &Pki::default(),
        policy(),
        fresh_list(&[0x99])
    )
    .is_ok());
    // No list yet: fail closed.
    assert_eq!(
        verify_with(
            &Spec::default(),
            &Pki::default(),
            policy(),
            Arc::new(RevocationList::new(86_400))
        )
        .err(),
        Some("revocation_list_unavailable")
    );
    // A list older than its grace period: fail closed.
    let stale = Arc::new(RevocationList::new(3600));
    stale.replace(HashSet::new(), NOW - 3601);
    assert_eq!(
        verify_with(&Spec::default(), &Pki::default(), policy(), stale).err(),
        Some("revocation_list_stale")
    );
    let within_grace = Arc::new(RevocationList::new(3600));
    within_grace.replace(HashSet::new(), NOW - 3600);
    assert!(verify_with(&Spec::default(), &Pki::default(), policy(), within_grace).is_ok());
}

#[test]
fn the_trait_rejects_without_saying_why() {
    let generated = generate(&Spec::default(), &Pki::default(), TRANSCRIPT);
    let verifier = verifier(&generated, policy(), fresh_list(&[]));
    assert!(verifier.verify(&generated.chain, TRANSCRIPT).is_ok());
    assert!(verifier
        .verify(&generated.chain, b"another transcript")
        .is_err());
    assert!(verifier.verify(&[], TRANSCRIPT).is_err());
    assert!(verifier.verify(&generated.chain[..1], TRANSCRIPT).is_err());
    assert!(verifier
        .verify(&vec![vec![0x30, 0x00]; 3], TRANSCRIPT)
        .is_err());
}

#[test]
fn google_roots_load_and_pin_both_keys() {
    let verifier = KeyAttestationVerifier::new(
        GOOGLE_ROOTS_PEM,
        policy(),
        Arc::new(RevocationList::new(1)),
        now,
    )
    .unwrap();
    assert_eq!(verifier.root_keys.len(), 2);
    assert!(
        KeyAttestationVerifier::new("", policy(), Arc::new(RevocationList::new(1)), now).is_err()
    );
}

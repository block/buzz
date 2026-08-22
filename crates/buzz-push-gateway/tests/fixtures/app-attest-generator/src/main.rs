use std::{
    env, fs,
    path::{Path, PathBuf},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use byteorder::{BigEndian, ByteOrder};
use ciborium::{cbor, Value};
use openssl::{
    asn1::{Asn1Integer, Asn1Object, Asn1OctetString, Asn1Time},
    bn::{BigNum, MsbOption},
    ec::{EcGroup, EcKey, PointConversionForm},
    hash::MessageDigest,
    nid::Nid,
    pkey::{PKey, Private},
    x509::{
        extension::{BasicConstraints, ExtendedKeyUsage, KeyUsage},
        X509Builder, X509Extension, X509Name, X509NameBuilder, X509,
    },
};
use sha2::{Digest, Sha256};

const APP_ID: &str = "TEAMID.xyz.buzz.mobile";
const CHALLENGE: &str = "buzz-app-attest-strict-verifier-fixture";
const CERT_VALIDITY_DAYS: u32 = 7_305;
const REGENERATION_COMMAND: &str = "cargo run --manifest-path crates/buzz-push-gateway/tests/fixtures/app-attest-generator/Cargo.toml -- --output-dir crates/buzz-push-gateway/tests/fixtures --good-aaguid appattest --wrong-aaguid appattestdevelop";

struct Args {
    output_dir: PathBuf,
    good_aaguid: String,
    wrong_aaguid: String,
}

struct CertificateAuthority {
    root_cert: X509,
    intermediate_cert: X509,
    intermediate_key: PKey<Private>,
}

struct Fixture {
    attestation_b64: String,
    key_id_b64: String,
    root_cert_pem: String,
    leaf_not_after: String,
}

fn main() {
    let args = parse_args();
    validate_aaguid(&args.good_aaguid);
    validate_aaguid(&args.wrong_aaguid);
    fs::create_dir_all(&args.output_dir).expect("create fixture output directory");

    let primary_ca = CertificateAuthority::generate("Buzz App Attest Fixture Root");
    let good = build_fixture(&primary_ca, &args.good_aaguid);
    let wrong_aaguid = build_fixture(&primary_ca, &args.wrong_aaguid);

    let unrelated_ca = CertificateAuthority::generate("Unrelated App Attest Fixture Root");
    let wrong_root = build_fixture(&unrelated_ca, &args.good_aaguid);

    write_fixture(
        &args.output_dir,
        "app-attest-good.json",
        "Valid synthetic Apple App Attest attestation for the gateway strict-verifier acceptance control.",
        &args.good_aaguid,
        &good,
    );
    write_fixture(
        &args.output_dir,
        "app-attest-wrong-aaguid.json",
        "Synthetic attestation with a development AAGUID and a correctly recomputed nonce; the strict verifier must report InvalidAAGUID.",
        &args.wrong_aaguid,
        &wrong_aaguid,
    );
    write_fixture(
        &args.output_dir,
        "app-attest-wrong-root.json",
        "Internally valid synthetic attestation signed by an unrelated root; the verifier configured with the good fixture root must reject it.",
        &args.good_aaguid,
        &wrong_root,
    );
}

fn parse_args() -> Args {
    let mut output_dir = None;
    let mut good_aaguid = None;
    let mut wrong_aaguid = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args
            .next()
            .unwrap_or_else(|| panic!("missing value for {arg}"));
        match arg.as_str() {
            "--output-dir" => output_dir = Some(PathBuf::from(value)),
            "--good-aaguid" => good_aaguid = Some(value),
            "--wrong-aaguid" => wrong_aaguid = Some(value),
            _ => panic!("unknown argument {arg}"),
        }
    }
    Args {
        output_dir: output_dir.expect("--output-dir is required"),
        good_aaguid: good_aaguid.expect("--good-aaguid is required"),
        wrong_aaguid: wrong_aaguid.expect("--wrong-aaguid is required"),
    }
}

fn validate_aaguid(aaguid: &str) {
    assert!(
        !aaguid.is_empty() && aaguid.len() <= 16 && aaguid.is_ascii(),
        "AAGUID must be 1 to 16 ASCII bytes"
    );
}

impl CertificateAuthority {
    fn generate(root_common_name: &str) -> Self {
        let root_key = p384_key();
        let root_name = name(root_common_name);
        let root_cert = certificate(
            &root_name,
            &root_name,
            &root_key,
            &root_key,
            MessageDigest::sha384(),
            true,
            None,
        );

        let intermediate_key = p256_key();
        let intermediate_name = name("Buzz App Attest Fixture Intermediate");
        let intermediate_cert = certificate(
            &intermediate_name,
            root_cert.subject_name(),
            &intermediate_key,
            &root_key,
            MessageDigest::sha384(),
            true,
            None,
        );

        Self {
            root_cert,
            intermediate_cert,
            intermediate_key,
        }
    }
}

fn build_fixture(ca: &CertificateAuthority, aaguid_value: &str) -> Fixture {
    let device_key = p256_key();
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).expect("P-256 group");
    let ec_key = device_key.ec_key().expect("device EC key");
    let mut context = openssl::bn::BigNumContext::new().expect("bignum context");
    let public_key = ec_key
        .public_key()
        .to_bytes(&group, PointConversionForm::UNCOMPRESSED, &mut context)
        .expect("serialize device public key");
    let key_id = Sha256::digest(&public_key);
    let key_id_b64 = STANDARD.encode(key_id);

    let mut aaguid = [0_u8; 16];
    aaguid[..aaguid_value.len()].copy_from_slice(aaguid_value.as_bytes());
    let mut auth_data = Vec::with_capacity(87);
    auth_data.extend_from_slice(&Sha256::digest(APP_ID.as_bytes()));
    auth_data.push(0x41);
    auth_data.extend_from_slice(&[0_u8; 4]);
    auth_data.extend_from_slice(&aaguid);
    let mut credential_length = [0_u8; 2];
    BigEndian::write_u16(&mut credential_length, key_id.len() as u16);
    auth_data.extend_from_slice(&credential_length);
    auth_data.extend_from_slice(&key_id);

    let mut nonce_input = auth_data.clone();
    nonce_input.extend_from_slice(&Sha256::digest(CHALLENGE.as_bytes()));
    let nonce = Sha256::digest(nonce_input);

    let leaf_name = name("Buzz App Attest Fixture Credential");
    let leaf_cert = certificate(
        &leaf_name,
        ca.intermediate_cert.subject_name(),
        &device_key,
        &ca.intermediate_key,
        MessageDigest::sha256(),
        false,
        Some(&nonce),
    );
    let leaf_der = leaf_cert.to_der().expect("encode credential certificate");
    let intermediate_der = ca
        .intermediate_cert
        .to_der()
        .expect("encode intermediate certificate");
    let value = cbor!({
        "fmt" => "apple-appattest",
        "attStmt" => {
            "x5c" => [
                Value::Bytes(leaf_der),
                Value::Bytes(intermediate_der)
            ],
            "receipt" => Value::Bytes(Vec::new())
        },
        "authData" => Value::Bytes(auth_data)
    })
    .expect("build attestation CBOR value");
    let mut cbor = Vec::new();
    ciborium::into_writer(&value, &mut cbor).expect("encode attestation CBOR");

    Fixture {
        attestation_b64: STANDARD.encode(cbor),
        key_id_b64,
        root_cert_pem: String::from_utf8(
            ca.root_cert.to_pem().expect("encode root certificate PEM"),
        )
        .expect("root PEM is UTF-8"),
        leaf_not_after: leaf_cert.not_after().to_string(),
    }
}

fn certificate(
    subject: &X509Name,
    issuer: &openssl::x509::X509NameRef,
    subject_key: &PKey<Private>,
    issuer_key: &PKey<Private>,
    signature_digest: MessageDigest,
    is_ca: bool,
    nonce: Option<&[u8]>,
) -> X509 {
    let mut builder = X509Builder::new().expect("create certificate builder");
    builder.set_version(2).expect("set X.509 version");
    builder
        .set_serial_number(&serial_number())
        .expect("set certificate serial");
    builder
        .set_subject_name(subject)
        .expect("set certificate subject");
    builder
        .set_issuer_name(issuer)
        .expect("set certificate issuer");
    builder
        .set_pubkey(subject_key)
        .expect("set certificate public key");
    builder
        .set_not_before(&Asn1Time::days_from_now(0).expect("build notBefore"))
        .expect("set notBefore");
    builder
        .set_not_after(&Asn1Time::days_from_now(CERT_VALIDITY_DAYS).expect("build long notAfter"))
        .expect("set notAfter");

    if is_ca {
        builder
            .append_extension(
                BasicConstraints::new()
                    .critical()
                    .ca()
                    .build()
                    .expect("build CA constraints"),
            )
            .expect("append CA constraints");
        builder
            .append_extension(
                KeyUsage::new()
                    .critical()
                    .key_cert_sign()
                    .crl_sign()
                    .build()
                    .expect("build CA key usage"),
            )
            .expect("append CA key usage");
    } else {
        builder
            .append_extension(
                BasicConstraints::new()
                    .critical()
                    .build()
                    .expect("build leaf constraints"),
            )
            .expect("append leaf constraints");
        builder
            .append_extension(
                ExtendedKeyUsage::new()
                    .other("1.2.840.113635.100.4.24")
                    .build()
                    .expect("build App Attest EKU"),
            )
            .expect("append App Attest EKU");

        let nonce = nonce.expect("credential certificate nonce");
        assert_eq!(nonce.len(), 32, "App Attest nonce must be 32 bytes");
        let mut extension_value = Vec::with_capacity(38);
        extension_value.extend_from_slice(&[0x30, 0x24, 0xa1, 0x22, 0x04, 0x20]);
        extension_value.extend_from_slice(nonce);
        let oid = Asn1Object::from_str("1.2.840.113635.100.8.2")
            .expect("parse App Attest nonce extension OID");
        let octets = Asn1OctetString::new_from_bytes(&extension_value)
            .expect("encode App Attest nonce extension");
        builder
            .append_extension(
                X509Extension::new_from_der(&oid, false, &octets)
                    .expect("build App Attest nonce extension"),
            )
            .expect("append App Attest nonce extension");
    }

    builder
        .sign(issuer_key, signature_digest)
        .expect("sign certificate");
    builder.build()
}

fn p256_key() -> PKey<Private> {
    ec_key(Nid::X9_62_PRIME256V1)
}

fn p384_key() -> PKey<Private> {
    ec_key(Nid::SECP384R1)
}

fn ec_key(curve: Nid) -> PKey<Private> {
    let group = EcGroup::from_curve_name(curve).expect("EC group");
    PKey::from_ec_key(EcKey::generate(&group).expect("generate EC key")).expect("wrap EC key")
}

fn name(common_name: &str) -> X509Name {
    let mut builder = X509NameBuilder::new().expect("create X.509 name builder");
    builder
        .append_entry_by_text("CN", common_name)
        .expect("set common name");
    builder
        .append_entry_by_text("O", "Buzz")
        .expect("set organization");
    builder.build()
}

fn serial_number() -> Asn1Integer {
    let mut number = BigNum::new().expect("create certificate serial");
    number
        .rand(128, MsbOption::MAYBE_ZERO, false)
        .expect("generate certificate serial");
    Asn1Integer::from_bn(&number).expect("convert certificate serial")
}

fn write_fixture(
    output_dir: &Path,
    file_name: &str,
    description: &str,
    aaguid: &str,
    fixture: &Fixture,
) {
    let generated_at = generation_date();
    let json = format!(
        concat!(
            "{{\n",
            "  \"description\": \"{}\",\n",
            "  \"generator\": \"crates/buzz-push-gateway/tests/fixtures/app-attest-generator\",\n",
            "  \"generated_at\": \"{}\",\n",
            "  \"regeneration_command\": \"{}\",\n",
            "  \"app_id\": \"{}\",\n",
            "  \"challenge\": \"{}\",\n",
            "  \"aaguid\": \"{}\",\n",
            "  \"leaf_not_after\": \"{}\",\n",
            "  \"attestation_b64\": \"{}\",\n",
            "  \"key_id_b64\": \"{}\",\n",
            "  \"root_cert_pem\": \"{}\"\n",
            "}}\n"
        ),
        json_escape(description),
        generated_at,
        json_escape(REGENERATION_COMMAND),
        APP_ID,
        CHALLENGE,
        aaguid,
        json_escape(&fixture.leaf_not_after),
        fixture.attestation_b64,
        fixture.key_id_b64,
        json_escape(&fixture.root_cert_pem),
    );
    fs::write(output_dir.join(file_name), json).expect("write fixture JSON");
}

fn generation_date() -> String {
    let output = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%d"])
        .output()
        .expect("run date for fixture metadata");
    assert!(output.status.success(), "date command failed");
    String::from_utf8(output.stdout)
        .expect("date output is UTF-8")
        .trim()
        .to_owned()
}

fn json_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            value if value.is_control() => {
                panic!("unsupported control character in fixture metadata")
            }
            value => vec![value],
        })
        .collect()
}

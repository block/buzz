# App Attest verifier fixtures

This standalone crate generates synthetic App Attest fixtures for the gateway's strict verifier tests. It has its own `[workspace]`, does not belong to the repository workspace dependency graph, and must never depend on `appattest`. That separation prevents the dependency's `testing` feature from being unified into the gateway test build, where it would permit the development AAGUID.

The generator owns the full root, intermediate, and credential certificate chain. It writes Apple's nonce extension OID `1.2.840.113635.100.8.2` and App Attest EKU `1.2.840.113635.100.4.24` directly. Generator correctness is therefore load-bearing. The strict verifier's good-fixture acceptance test is the encoding oracle. Any generator change must pass the full App Attest test floor, and a fixture rejected by the shipped verifier must never be made green by loosening a test.

`apple-app-attestation-root.pem` is the production pin-control fixture downloaded from [Apple's certificate authority](https://www.apple.com/certificateauthority/Apple_App_Attestation_Root_CA.pem). Its exact PEM-file SHA-256 must remain `c778d09ac341f7fd9f8f3b19e2b815af6aed4ad4490e1e92c05cb355212a5013`.

Regenerate from the repository root:

```bash
cargo run --manifest-path crates/buzz-push-gateway/tests/fixtures/app-attest-generator/Cargo.toml -- \
  --output-dir crates/buzz-push-gateway/tests/fixtures \
  --good-aaguid appattest \
  --wrong-aaguid appattestdevelop
```

The command rewrites `app-attest-good.json`, `app-attest-wrong-aaguid.json`, and `app-attest-wrong-root.json`. Review all fixture changes and run the complete gateway package test suite afterward.

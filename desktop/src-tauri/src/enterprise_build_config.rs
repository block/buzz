//! One build-consumer contract, called by build.rs and tested in the native suite.
pub(crate) fn compile_config(raw: &str, system_keyring: bool) -> Result<String, String> {
    if !system_keyring {
        return Err("Corporate builds require system-keyring".into());
    }
    let config: buzz_ws_client_pkg::enterprise_oauth::EnterpriseLoginConfig =
        serde_json::from_str(raw).map_err(|_| "Invalid corporate build configuration")?;
    config.validate()?;
    // Sorted JSON matches the release verifier, unlike declaration-order serialization.
    let value = serde_json::to_value(config).map_err(|_| "Invalid corporate config")?;
    serde_json::to_string(&value).map_err(|_| "Invalid corporate config".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    const VALID: &str = r#"{"signerUrl":"https://signer.example/cash-app/goose/","issuer":"https://issuer.example/","clientId":"native-client","audience":"https://signer-api.example","organization":"org_corp","connection":"corporate","redirectUri":"http://127.0.0.1:45871/enterprise-callback"}"#;
    #[test]
    fn actual_build_consumer_requires_keyring_and_exact_native_schema() {
        assert!(compile_config(VALID, false).is_err());
        let canonical = compile_config(VALID, true).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&canonical).unwrap();
        assert_eq!(parsed.as_object().unwrap().len(), 7);
        assert_eq!(
            parsed["redirectUri"],
            "http://127.0.0.1:45871/enterprise-callback"
        );
        assert!(canonical.starts_with("{\"audience\":"));
        for invalid in [
            VALID.replace("45871", "0"),
            VALID.replace("http://127.0.0.1", "http://localhost"),
            VALID.replace("https://signer.example", "http://signer.example"),
            VALID.replace("cash-app/goose/", "cash-app/goose/v1/buzz/identity/sign"),
            VALID.replace("\"issuer\":", "\"environment\":\"staging\",\"issuer\":"),
            VALID.replace(
                "\"issuer\":",
                "\"clientSecret\":\"not-allowed\",\"issuer\":",
            ),
            VALID.replace("\"issuer\":", "\"clientId\":\"duplicate\",\"issuer\":"),
        ] {
            assert!(compile_config(&invalid, true).is_err());
        }
    }
}

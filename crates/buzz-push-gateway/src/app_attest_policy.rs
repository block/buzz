//! Selects the production Apple verifier or the feature-gated development stub.

use crate::app_attest::{
    AppAttestError, AppAttestVerifier, VerifiedAssertion, VerifiedAttestation,
};
#[cfg(feature = "dev-app-attest-bypass")]
#[derive(Clone)]
pub struct DevelopmentAppAttestPolicy {
    _private: (),
}

#[cfg(feature = "dev-app-attest-bypass")]
pub struct DevelopmentAppAttestBypass {
    _private: (),
}

#[cfg(feature = "dev-app-attest-bypass")]
impl DevelopmentAppAttestBypass {
    pub(crate) fn enabled() -> Self {
        Self { _private: () }
    }
}

#[cfg_attr(
    not(feature = "dev-app-attest-bypass"),
    doc = r#"
The development policy is structurally unavailable in default builds:

```compile_fail
use buzz_push_gateway::app_attest_policy::AppAttestPolicy;

let _ = AppAttestPolicy::Development;
```
"#
)]
#[derive(Clone)]
pub enum AppAttestPolicy {
    Apple(AppAttestVerifier),
    #[cfg(feature = "dev-app-attest-bypass")]
    Development(DevelopmentAppAttestPolicy),
}

impl AppAttestPolicy {
    pub fn apple(verifier: AppAttestVerifier) -> Self {
        Self::Apple(verifier)
    }

    #[cfg(feature = "dev-app-attest-bypass")]
    pub fn from_config(
        bypass: Option<DevelopmentAppAttestBypass>,
        apple: AppAttestVerifier,
    ) -> Self {
        if bypass.is_some() {
            tracing::warn!(
                "DEVELOPMENT APP ATTEST BYPASS ACTIVE; Apple attestation and assertion verification are disabled"
            );
            Self::development()
        } else {
            Self::apple(apple)
        }
    }

    #[cfg(feature = "dev-app-attest-bypass")]
    fn development() -> Self {
        Self::Development(DevelopmentAppAttestPolicy { _private: () })
    }

    pub fn verify_attestation(
        &self,
        attestation_b64: &str,
        key_id_b64: &str,
        client_data: &[u8],
    ) -> Result<VerifiedAttestation, AppAttestError> {
        match self {
            Self::Apple(verifier) => {
                verifier.verify_attestation(attestation_b64, key_id_b64, client_data)
            }
            #[cfg(feature = "dev-app-attest-bypass")]
            Self::Development(_) => {
                crate::dev_app_attest::verify_attestation(attestation_b64, key_id_b64, client_data)
            }
        }
    }

    pub fn verify_assertion(
        &self,
        assertion_b64: &str,
        client_data: &[u8],
        public_key: &[u8],
        previous_counter: u32,
        challenge: &str,
        stored_challenge: &str,
    ) -> Result<VerifiedAssertion, AppAttestError> {
        match self {
            Self::Apple(verifier) => verifier.verify_assertion(
                assertion_b64,
                client_data,
                public_key,
                previous_counter,
                challenge,
                stored_challenge,
            ),
            #[cfg(feature = "dev-app-attest-bypass")]
            Self::Development(_) => crate::dev_app_attest::verify_assertion(
                assertion_b64,
                client_data,
                public_key,
                previous_counter,
                challenge,
                stored_challenge,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn development_bypass_stays_a_non_default_feature() {
        assert!(include_str!("../Cargo.toml")
            .contains("[features]\ndefault = []\ndev-app-attest-bypass = []"));
    }
}

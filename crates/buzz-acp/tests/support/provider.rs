//! Real `daz-secrets` provider entries for process-boundary tests. Identity
//! bytes travel only through the provider; the harness under test receives the
//! non-secret service/account coordinates.
#![allow(dead_code)]

pub struct ProviderIdentity {
    client: daz_secrets::BlockingClient,
    pub service: String,
    pub account: String,
}

impl ProviderIdentity {
    /// Store `secret` (hex or nsec) under a unique account of the test service.
    pub fn seed(secret: &str) -> Self {
        let client =
            daz_secrets::BlockingClient::from_default_config().expect("daz-secrets test provider");
        let service = "buzz-acp-tests".to_string();
        let account = format!("pid-{}-{}", std::process::id(), uuid::Uuid::new_v4());
        client
            .set(&service, &account, secret.as_bytes(), None)
            .expect("seed provider identity");
        Self {
            client,
            service,
            account,
        }
    }

    pub fn for_keys(keys: &nostr::Keys) -> Self {
        Self::seed(&keys.secret_key().to_secret_hex())
    }

    /// CLI arguments selecting this entry.
    pub fn args(&self) -> [&str; 4] {
        [
            "--secret-service",
            &self.service,
            "--secret-account",
            &self.account,
        ]
    }
}

impl Drop for ProviderIdentity {
    fn drop(&mut self) {
        let _ = self.client.delete(&self.service, &self.account, None);
    }
}

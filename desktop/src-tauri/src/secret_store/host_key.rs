//! Host key creation uses the same cross-process keychain transaction as agents.
impl super::SecretStore {
    /// Atomically keep an existing secret or insert a candidate. Never overwrite.
    pub(crate) fn host_key(&self, name: &str) -> Result<String, String> {
        #[cfg(feature = "system-keyring")]
        {
            let candidate = nostr::Keys::generate().secret_key().to_secret_hex();
            let mut result = String::new();
            self.mutate_blob(|blob| {
                result = blob.entry(name.to_owned()).or_insert(candidate).clone();
            })?;
            Ok(result)
        }
        #[cfg(not(feature = "system-keyring"))]
        {
            let _ = name;
            Err("Host registration requires secure key storage in this build".into())
        }
    }
}

use serde::{Deserialize, Serialize};

const KEYRING_KEY: &str = "workbench_oidc_user_v1";
const MAX_KEY_LENGTH: usize = 512;
const MAX_VALUE_LENGTH: usize = 64 * 1024;

#[derive(Debug, Serialize, Deserialize)]
struct StoredUser {
    key: String,
    value: String,
}

fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > MAX_KEY_LENGTH || !key.starts_with("buzz.oidc.user.user:") {
        return Err("invalid Workbench OIDC user key".into());
    }
    Ok(())
}

fn store() -> &'static crate::secret_store::SecretStore {
    crate::secret_store::SecretStore::shared(crate::app_state::keyring_service())
}

#[tauri::command]
pub fn workbench_oidc_user_load(key: String) -> Result<Option<String>, String> {
    validate_key(&key)?;
    let Some(raw) = store().load(KEYRING_KEY)? else {
        return Ok(None);
    };
    let saved: StoredUser = serde_json::from_str(&raw)
        .map_err(|_| "stored Workbench OIDC user is invalid".to_string())?;
    Ok((saved.key == key).then_some(saved.value))
}

#[tauri::command]
pub fn workbench_oidc_user_save(key: String, value: String) -> Result<(), String> {
    validate_key(&key)?;
    if value.is_empty() || value.len() > MAX_VALUE_LENGTH {
        return Err("invalid Workbench OIDC user value".into());
    }
    let raw = serde_json::to_string(&StoredUser { key, value })
        .map_err(|_| "could not encode Workbench OIDC user".to_string())?;
    store().store(KEYRING_KEY, &raw)
}

#[tauri::command]
pub fn workbench_oidc_user_delete(key: String) -> Result<(), String> {
    validate_key(&key)?;
    if workbench_oidc_user_load(key)?.is_some() {
        store().delete(KEYRING_KEY)?;
    }
    Ok(())
}

#[tauri::command]
pub fn workbench_oidc_user_keys() -> Result<Vec<String>, String> {
    let Some(raw) = store().load(KEYRING_KEY)? else {
        return Ok(Vec::new());
    };
    let saved: StoredUser = serde_json::from_str(&raw)
        .map_err(|_| "stored Workbench OIDC user is invalid".to_string())?;
    validate_key(&saved.key)?;
    Ok(vec![saved.key])
}

#[cfg(test)]
mod tests {
    use super::validate_key;

    #[test]
    fn accepts_only_oidc_user_keys() {
        assert!(validate_key("buzz.oidc.user.user:https://issuer:client").is_ok());
        assert!(validate_key("user:https://issuer:client").is_err());
        assert!(validate_key("state:opaque").is_err());
        assert!(validate_key("").is_err());
    }
}

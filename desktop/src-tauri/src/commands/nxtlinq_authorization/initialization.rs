use super::*;
use ed25519_dalek::{pkcs8::DecodePrivateKey, SigningKey};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
};
use zeroize::Zeroizing;

pub(super) fn resolve_real_project_root(project_root: &str) -> Result<PathBuf, String> {
    let requested = Path::new(project_root.trim());
    validate_absolute_path(requested, "project root")?;
    let metadata = std::fs::symlink_metadata(requested)
        .map_err(|_| "project root does not exist or cannot be inspected".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("project root must be a real directory, not a symlink".to_string());
    }
    std::fs::canonicalize(requested).map_err(|_| "project root could not be resolved".to_string())
}

pub(super) fn initialization_status(project_root: &str) -> NxtlinqAttestInitializationStatus {
    let Ok(project) = resolve_real_project_root(project_root) else {
        return NxtlinqAttestInitializationStatus {
            status: NxtlinqAttestInitializationState::Invalid,
            detail: Some("Choose an existing project using an absolute path.".into()),
        };
    };
    let nxtlinq = project.join("nxtlinq");
    let metadata = match std::fs::symlink_metadata(&nxtlinq) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return NxtlinqAttestInitializationStatus {
                status: NxtlinqAttestInitializationState::Missing,
                detail: None,
            };
        }
        Err(_) => {
            return NxtlinqAttestInitializationStatus {
                status: NxtlinqAttestInitializationState::Invalid,
                detail: Some("The project's nxtlinq directory cannot be inspected.".into()),
            };
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return NxtlinqAttestInitializationStatus {
            status: NxtlinqAttestInitializationState::Invalid,
            detail: Some("The project's nxtlinq path must be a real directory.".into()),
        };
    }
    match std::fs::symlink_metadata(nxtlinq.join("private.key")) {
        Ok(_) => {
            return NxtlinqAttestInitializationStatus {
                status: NxtlinqAttestInitializationState::WorkspacePrivateKey,
                detail: Some(
                    "Move nxtlinq/private.key to owner-controlled storage outside the Agent workspace."
                        .into(),
                ),
            };
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return NxtlinqAttestInitializationStatus {
                status: NxtlinqAttestInitializationState::Invalid,
                detail: Some("The project private-key location cannot be inspected.".into()),
            };
        }
    }
    for name in ["public.key", "agent.manifest.json"] {
        let Ok(metadata) = std::fs::symlink_metadata(nxtlinq.join(name)) else {
            return NxtlinqAttestInitializationStatus {
                status: NxtlinqAttestInitializationState::Invalid,
                detail: Some("The existing Nxtlinq initialization is incomplete.".into()),
            };
        };
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || (name == "public.key" && metadata.len() > 64 * 1024)
        {
            return NxtlinqAttestInitializationStatus {
                status: NxtlinqAttestInitializationState::Invalid,
                detail: Some("Nxtlinq initialization files are not safe to inspect.".into()),
            };
        }
    }
    let project_public_key =
        read_bounded_real_file(&nxtlinq.join("public.key"), 64 * 1024, "project public key")
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok());
    let valid = read_bounded_real_file(
        &nxtlinq.join("agent.manifest.json"),
        MAX_MANIFEST_BYTES,
        "project manifest",
    )
    .ok()
    .and_then(|bytes| serde_json::from_slice::<Map<String, Value>>(&bytes).ok())
    .and_then(|document| {
        validate_initialized_manifest(&document, project_public_key.as_deref()?).ok()
    })
    .is_some();
    if valid {
        NxtlinqAttestInitializationStatus {
            status: NxtlinqAttestInitializationState::Initialized,
            detail: None,
        }
    } else {
        NxtlinqAttestInitializationStatus {
            status: NxtlinqAttestInitializationState::Invalid,
            detail: Some("The Nxtlinq manifest is invalid or cannot be inspected.".into()),
        }
    }
}

fn valid_hash_or_init_placeholder(value: &str) -> bool {
    value == "<set by attest sign>"
        || (value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
}

fn validate_initialized_manifest(
    document: &Map<String, Value>,
    project_public_key: &str,
) -> Result<(String, String), String> {
    for field in ["name", "version"] {
        if !document
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(format!(
                "Nxtlinq manifest field {field} is missing or empty"
            ));
        }
    }
    if !document
        .get("scope")
        .and_then(Value::as_array)
        .is_some_and(|scope| {
            !scope.is_empty()
                && scope
                    .iter()
                    .all(|entry| entry.as_str().is_some_and(|value| !value.trim().is_empty()))
        })
    {
        return Err("Nxtlinq manifest scope must contain non-empty strings".to_string());
    }
    if !document
        .get("issuedAt")
        .and_then(Value::as_u64)
        .is_some_and(|issued_at| issued_at > 0)
    {
        return Err("Nxtlinq manifest issuedAt is missing or invalid".to_string());
    }
    for field in ["contentHash", "artifactHash"] {
        if !document
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(valid_hash_or_init_placeholder)
        {
            return Err(format!("Nxtlinq manifest field {field} is invalid"));
        }
    }
    if document.get("attestCliVersion").and_then(Value::as_str) != Some(NXTLINQ_ATTEST_VERSION) {
        return Err("Nxtlinq manifest was not initialized by the reviewed Attest version".into());
    }
    let signer_key_id = validate_signer_key_id(
        document
            .get("signerKeyId")
            .and_then(Value::as_str)
            .ok_or("Nxtlinq manifest signerKeyId is missing")?,
    )?
    .to_string();
    let embedded_public_key = document
        .get("publicKey")
        .and_then(Value::as_str)
        .ok_or("Nxtlinq manifest publicKey is missing")?;
    if project_public_key.trim() != embedded_public_key.trim() {
        return Err("Nxtlinq public.key does not match the manifest identity".to_string());
    }
    let fingerprint = public_key_fingerprint(embedded_public_key)?;
    Ok((signer_key_id, fingerprint))
}

fn public_key_fingerprint(public_key_pem: &str) -> Result<String, String> {
    let key = VerifyingKey::from_public_key_pem(public_key_pem)
        .map_err(|_| "initialized Nxtlinq public key is not valid Ed25519 PEM".to_string())?;
    let der = key
        .to_public_key_der()
        .map_err(|_| "initialized Nxtlinq public key cannot be encoded".to_string())?;
    Ok(format!("sha256:{}", sha256_hex(der.as_bytes())))
}

fn run_attest_init(current_dir: &Path, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let (node, cli) = managed_attest_cli()?;
    let mut command = std::process::Command::new(&node);
    command.arg(&cli).arg("init");
    for arg in args {
        command.arg(arg);
    }
    let output = command
        .current_dir(current_dir)
        .output()
        .map_err(|error| format!("launch managed Nxtlinq Attest project initializer: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_error_without_paths(
            "Nxtlinq project initialization",
            &output,
            &[current_dir, &cli],
        ))
    }
}

fn policy_key_suffix(fingerprint: &str) -> &str {
    fingerprint.strip_prefix("sha256:").unwrap_or(fingerprint)
}

pub(super) fn generated_trust_store_document(key_id: &str, public_key: &str) -> Value {
    serde_json::json!({
        "trustedSigners": [{
            "keyId": key_id,
            "publicKey": public_key.trim()
        }]
    })
}

fn write_generated_trust_store(
    app: &AppHandle,
    project: &Path,
    key_id: &str,
    public_key: &str,
    fingerprint: &str,
) -> Result<PathBuf, String> {
    let root = config_root(app)?.join("trusted-signers");
    std::fs::create_dir_all(&root)
        .map_err(|_| "create protected Nxtlinq trust-store directory".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "restrict Nxtlinq trust-store directory".to_string())?;
    }
    let root = std::fs::canonicalize(&root)
        .map_err(|_| "resolve protected Nxtlinq trust-store directory".to_string())?;
    if root.starts_with(project) {
        return Err("Buzz trust storage must remain outside the Agent workspace".to_string());
    }
    let path = root.join(format!("{}.json", policy_key_suffix(fingerprint)));
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err("Buzz-managed trust store must be a real file".to_string()),
        Err(_) => return Err("inspect Buzz-managed trust store".to_string()),
    }
    let payload = serde_json::to_vec_pretty(&generated_trust_store_document(key_id, public_key))
        .map_err(|_| "serialize generated Nxtlinq trust store".to_string())?;
    atomic_write_json_restricted(&path, &payload)
        .map_err(|_| "write protected Nxtlinq trust store".to_string())?;
    Ok(path)
}

fn select_generated_trust_store(app: &AppHandle, path: &Path) -> Result<(), String> {
    let mut config = load_authorization_config(app)?;
    config.trust_store = Some(path.display().to_string());
    save_authorization_config(app, config)?;
    Ok(())
}

fn policy_keyring_name(fingerprint: &str) -> String {
    format!("nxtlinq-policy:{}", policy_key_suffix(fingerprint))
}

fn policy_fallback_path(app: &AppHandle, fingerprint: &str) -> Result<PathBuf, String> {
    Ok(config_root(app)?
        .join("keys")
        .join(format!("{}.private.pem", policy_key_suffix(fingerprint))))
}

fn validate_private_key_for_public(
    private_key_pem: &str,
    public_key_pem: &str,
) -> Result<(), String> {
    let signing_key = SigningKey::from_pkcs8_pem(private_key_pem)
        .map_err(|_| "managed Nxtlinq private key is not valid Ed25519 PKCS#8 PEM".to_string())?;
    let public_key = VerifyingKey::from_public_key_pem(public_key_pem)
        .map_err(|_| "project Nxtlinq public key is not valid Ed25519 PEM".to_string())?;
    if signing_key.verifying_key() != public_key {
        return Err(
            "managed Nxtlinq private key does not match the project public key".to_string(),
        );
    }
    Ok(())
}

enum StoredPolicyKey {
    Keyring { name: String },
    Fallback { path: PathBuf },
}

impl StoredPolicyKey {
    fn label(&self) -> &'static str {
        match self {
            Self::Keyring { .. } => "System secure storage",
            Self::Fallback { .. } => "Protected Buzz storage (keyring unavailable)",
        }
    }

    fn rollback(self) {
        match self {
            Self::Keyring { name } => {
                let store =
                    crate::secret_store::SecretStore::shared(crate::app_state::keyring_service());
                let _ = store.delete(&name);
            }
            Self::Fallback { path } => {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn store_managed_private_key(
    app: &AppHandle,
    project: &Path,
    fingerprint: &str,
    private_key_pem: &str,
) -> Result<StoredPolicyKey, String> {
    let name = policy_keyring_name(fingerprint);
    let store = crate::secret_store::SecretStore::shared(crate::app_state::keyring_service());
    let keyring_result = (|| -> Result<(), String> {
        if let Some(existing) = store.load(&name)? {
            if existing != private_key_pem {
                return Err("a different Nxtlinq key already uses this fingerprint".to_string());
            }
            return Ok(());
        }
        store.store(&name, private_key_pem)?;
        match store.load(&name)? {
            Some(stored) if stored == private_key_pem => Ok(()),
            _ => Err("Nxtlinq keyring read-back verification failed".to_string()),
        }
    })();
    if keyring_result.is_ok() {
        return Ok(StoredPolicyKey::Keyring { name });
    }

    let path = policy_fallback_path(app, fingerprint)?;
    let directory = path
        .parent()
        .ok_or("Nxtlinq fallback key path has no parent directory")?;
    std::fs::create_dir_all(directory)
        .map_err(|_| "create protected Nxtlinq fallback key directory".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "restrict Nxtlinq fallback key directory".to_string())?;
    }
    let directory = std::fs::canonicalize(directory)
        .map_err(|_| "resolve Nxtlinq fallback key directory".to_string())?;
    if directory.starts_with(project) {
        return Err("Buzz secret storage must remain outside the Agent workspace".to_string());
    }
    write_new_private_file(&path, private_key_pem.as_bytes())?;
    Ok(StoredPolicyKey::Fallback { path })
}

pub(super) fn initialize_attest_with_generated_identity(
    app: &AppHandle,
    project_root: &str,
    key_id: &str,
) -> Result<NxtlinqAttestInitializationResult, String> {
    let key_id = validate_signer_key_id(key_id)?;
    let project = resolve_real_project_root(project_root)?;
    let destination = project.join("nxtlinq");
    match std::fs::symlink_metadata(&destination) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(
                "this project already contains a Nxtlinq initialization; inspect it before continuing"
                    .to_string(),
            );
        }
        Err(_) => {
            return Err("the Nxtlinq initialization destination cannot be inspected".to_string());
        }
    }
    let project_parent = project
        .parent()
        .ok_or("project root has no usable parent directory")?;
    let staging = tempfile::Builder::new()
        .prefix(".nxtlinq-attest-init-")
        .tempdir_in(project_parent)
        .map_err(|_| "create protected Nxtlinq initialization staging directory".to_string())?;
    // Run Attest's standard first-run flow while the Agent is stopped. Attest
    // is the authority that generates both keys and the manifest. Staging is
    // mode 0700 and lives beside the project, so a crash cannot strand a
    // private key in the Agent workspace and the final public-only install can
    // be an atomic rename.
    run_attest_init(staging.path(), &[])?;
    let staged_nxtlinq = staging.path().join("nxtlinq");
    let staged_private_key =
        validate_external_private_key(&project, &staged_nxtlinq.join("private.key"))?;
    let generated_private_key = Zeroizing::new(
        String::from_utf8(read_bounded_real_file(
            &staged_private_key,
            64 * 1024,
            "Attest-generated private key",
        )?)
        .map_err(|_| "Attest-generated private key is not UTF-8 PEM".to_string())?,
    );
    let staged_public_key = read_bounded_real_file(
        &staged_nxtlinq.join("public.key"),
        64 * 1024,
        "initialized public key",
    )?;
    let staged_manifest = read_bounded_real_file(
        &staged_nxtlinq.join("agent.manifest.json"),
        MAX_MANIFEST_BYTES,
        "initialized manifest",
    )?;
    let staged_public_key_text = std::str::from_utf8(&staged_public_key)
        .map_err(|_| "initialized public key is not UTF-8 PEM".to_string())?;
    validate_private_key_for_public(&generated_private_key, staged_public_key_text)?;
    let expected_fingerprint = public_key_fingerprint(staged_public_key_text)?;
    let mut manifest: Map<String, Value> = serde_json::from_slice(&staged_manifest)
        .map_err(|_| "initialized Nxtlinq manifest is not valid JSON".to_string())?;
    // Standard Attest init intentionally uses its local-development identity
    // label. The Desktop owner supplies the operational key ID that must also
    // match the operator trust-store entry.
    manifest.insert("signerKeyId".to_string(), Value::String(key_id.to_string()));
    let staged_manifest_payload = serde_json::to_vec_pretty(&manifest)
        .map_err(|_| "serialize initialized Nxtlinq manifest".to_string())?;
    std::fs::write(
        staged_nxtlinq.join("agent.manifest.json"),
        staged_manifest_payload,
    )
    .map_err(|_| "write initialized Nxtlinq signer key ID".to_string())?;
    let (initialized_key_id, initialized_fingerprint) =
        validate_initialized_manifest(&manifest, staged_public_key_text)?;
    if initialized_key_id != key_id {
        return Err("initialized Nxtlinq manifest has an unexpected signer key ID".to_string());
    }
    if initialized_fingerprint != expected_fingerprint {
        return Err("initialized Nxtlinq identity does not match the generated key".to_string());
    }

    let stored_key =
        store_managed_private_key(app, &project, &expected_fingerprint, &generated_private_key)?;
    let private_key_storage = stored_key.label().to_string();

    // The managed copy must be durable before removing Attest's project-local
    // source key. Failure to remove it aborts and rolls back managed storage;
    // no workspace state has been installed yet.
    if std::fs::remove_file(&staged_private_key).is_err()
        || std::fs::symlink_metadata(&staged_private_key).is_ok()
    {
        stored_key.rollback();
        return Err("Attest private key could not be removed from project initialization".into());
    }

    // This explicit Desktop owner ceremony is also the trust-enrollment
    // boundary for the local deployment MVP. Copy the verified public key into
    // Buzz-owned app data; never reference the Agent-writable project key from
    // the operator trust store.
    let trust_store_path = match write_generated_trust_store(
        app,
        &project,
        key_id,
        staged_public_key_text,
        &expected_fingerprint,
    ) {
        Ok(path) => path,
        Err(error) => {
            stored_key.rollback();
            return Err(error);
        }
    };

    // The staged directory now contains only public material. Renaming it into
    // place is atomic because it was created beside the project.
    if std::fs::rename(&staged_nxtlinq, &destination).is_err() {
        stored_key.rollback();
        return Err("the public project initialization could not be installed".to_string());
    }
    select_generated_trust_store(app, &trust_store_path)?;
    Ok(NxtlinqAttestInitializationResult {
        cancelled: false,
        signer_key_id: Some(key_id.to_string()),
        public_key_fingerprint: Some(expected_fingerprint),
        private_key_storage: Some(private_key_storage),
        trust_store_path: Some(trust_store_path.display().to_string()),
    })
}

pub(super) fn load_managed_private_key(
    app: &AppHandle,
    project_root: &str,
) -> Result<Zeroizing<String>, String> {
    let project = resolve_real_project_root(project_root)?;
    let public_key_bytes = read_bounded_real_file(
        &project.join("nxtlinq/public.key"),
        64 * 1024,
        "project public key",
    )?;
    let public_key = std::str::from_utf8(&public_key_bytes)
        .map_err(|_| "project public key is not UTF-8 PEM".to_string())?;
    let fingerprint = public_key_fingerprint(public_key)?;
    let name = policy_keyring_name(&fingerprint);
    let store = crate::secret_store::SecretStore::shared(crate::app_state::keyring_service());
    let keyring_error = match store.load(&name) {
        Ok(Some(secret)) => {
            validate_private_key_for_public(&secret, public_key)?;
            return Ok(Zeroizing::new(secret));
        }
        Ok(None) => None,
        Err(error) => Some(error),
    };

    let fallback = policy_fallback_path(app, &fingerprint)?;
    if fallback.exists() {
        let fallback = validate_external_private_key(&project, &fallback)?;
        let bytes = read_bounded_real_file(&fallback, 64 * 1024, "managed private key")?;
        let secret = String::from_utf8(bytes)
            .map_err(|_| "managed private key is not UTF-8 PEM".to_string())?;
        validate_private_key_for_public(&secret, public_key)?;
        return Ok(Zeroizing::new(secret));
    }

    Err(if keyring_error.is_some() {
        "The system keyring is unavailable and no protected fallback key exists. Unlock the keyring and retry."
            .to_string()
    } else {
        "No Buzz-managed private key matches this project. Reinitialize the project or use an external signing workflow."
            .to_string()
    })
}

fn read_bounded_real_file(path: &Path, limit: usize, label: &str) -> Result<Vec<u8>, String> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| format!("{label} cannot be inspected"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() as usize > limit {
        return Err(format!("{label} is not a safe regular file"));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|_| format!("{label} cannot be opened safely"))?;
    let opened_metadata = file
        .metadata()
        .map_err(|_| format!("{label} cannot be inspected after opening"))?;
    if !opened_metadata.is_file() || opened_metadata.len() as usize > limit {
        return Err(format!("{label} is not a safe regular file"));
    }
    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("{label} cannot be read"))?;
    if bytes.len() > limit {
        return Err(format!("{label} is too large"));
    }
    Ok(bytes)
}

pub(super) fn write_new_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path).map_err(|error| {
        format!("create the external private-key file without overwriting it: {error}")
    })?;
    file.write_all(bytes)
        .map_err(|_| "write the external private-key file".to_string())?;
    file.sync_all()
        .map_err(|_| "sync the external private-key file".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| "restrict the external private-key file".to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_private_key_is_owner_only_and_never_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let key = root.path().join("managed.private.pem");

        write_new_private_file(&key, b"first").unwrap();
        assert!(write_new_private_file(&key, b"second").is_err());
        assert_eq!(std::fs::read(&key).unwrap(), b"first");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&key).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn managed_key_name_is_deterministic_and_does_not_contain_the_project_path() {
        let fingerprint = "sha256:5fb2c4b87f0a";
        let first = policy_keyring_name(fingerprint);
        let second = policy_keyring_name(fingerprint);

        assert_eq!(first, second);
        assert!(first.starts_with("nxtlinq-policy:"));
        assert!(!first.contains('/'));
        assert!(!first.contains("sha256:"));
    }
}

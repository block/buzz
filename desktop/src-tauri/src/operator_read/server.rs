/// Start the owner-only Unix-socket service inside the signed Desktop process.
pub fn start_operator_read_server(app: tauri::AppHandle) -> Result<(), String> {
    if !is_production_bundle(&app) {
        return Ok(());
    }
    let state = app.state::<AppState>();
    if capture_production_keys(&state).is_err() {
        return Err(
            "operator reads require Block's signed production app and keyring-backed identity"
                .to_string(),
        );
    }
    let socket_path = resolve_socket_path().map_err(|error| error.message.to_string())?;
    prepare_socket_path(&socket_path).map_err(|error| error.message.to_string())?;
    let listener = UnixListener::bind(&socket_path)
        .map_err(|error| format!("could not bind owner-only socket: {error}"))?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("could not protect owner-only socket: {error}"))?;
    let metadata = validate_socket(&socket_path).map_err(|error| error.message.to_string())?;
    let cleanup = SocketCleanup {
        path: socket_path,
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    let replay_guard = Arc::new(ReplayGuard::default());

    tauri::async_runtime::spawn(async move {
        let _cleanup = cleanup;
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                eprintln!("buzz-desktop: operator read service stopped accepting requests");
                break;
            };
            let app = app.clone();
            let replay_guard = replay_guard.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = handle_connection(stream, app, replay_guard).await {
                    eprintln!("buzz-desktop: operator read request failed: {}", error.code);
                }
            });
        }
    });
    Ok(())
}

pub fn is_production_bundle(app: &tauri::AppHandle) -> bool {
    app.config().identifier == PRODUCTION_BUNDLE_IDENTIFIER
}

/// Return whether this process is the trusted production credential owner.
pub fn is_trusted_production_owner(app: &tauri::AppHandle) -> bool {
    let state = app.state::<AppState>();
    is_production_bundle(app) && capture_production_keys(&state).is_ok()
}

fn production_credential_owner_allowed(
    identifier: &str,
    storage: IdentityStorage,
    code_signature_valid: bool,
) -> bool {
    if identifier != PRODUCTION_BUNDLE_IDENTIFIER || !code_signature_valid {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        storage == IdentityStorage::SystemKeyring
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = storage;
        false
    }
}

#[cfg(target_os = "macos")]
fn production_code_signature_valid() -> bool {
    use security_framework::os::macos::code_signing::{Flags, SecCode, SecRequirement};

    let Ok(requirement) = PRODUCTION_CODE_REQUIREMENT.parse::<SecRequirement>() else {
        return false;
    };
    let Ok(code) = SecCode::for_self(Flags::NONE) else {
        return false;
    };
    code.check_validity(
        Flags::STRICT_VALIDATE | Flags::CHECK_TRUSTED_ANCHORS | Flags::NO_NETWORK_ACCESS,
        &requirement,
    )
    .is_ok()
}

#[cfg(not(target_os = "macos"))]
fn production_code_signature_valid() -> bool {
    false
}

#[cfg(target_os = "macos")]
fn production_process_signature_valid(pid: libc::pid_t) -> bool {
    use security_framework::os::macos::code_signing::{
        Flags, GuestAttributes, SecCode, SecRequirement,
    };

    let Ok(requirement) = PRODUCTION_CODE_REQUIREMENT.parse::<SecRequirement>() else {
        return false;
    };
    let mut attributes = GuestAttributes::new();
    attributes.set_pid(pid);
    let Ok(code) = SecCode::copy_guest_with_attribues(None, &attributes, Flags::NONE) else {
        return false;
    };
    code.check_validity(
        Flags::STRICT_VALIDATE | Flags::CHECK_TRUSTED_ANCHORS | Flags::NO_NETWORK_ACCESS,
        &requirement,
    )
    .is_ok()
}

#[cfg(not(target_os = "macos"))]
fn production_process_signature_valid(_pid: libc::pid_t) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn connected_peer_pid(stream: &StdUnixStream) -> Result<u32, OperatorError> {
    use std::os::fd::AsRawFd;

    let mut pid: libc::pid_t = 0;
    let mut length = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: the pointers reference a writable pid_t and its exact size for
    // the duration of getsockopt; `stream` owns a valid Unix-domain socket fd.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            std::ptr::addr_of_mut!(pid).cast(),
            std::ptr::addr_of_mut!(length),
        )
    };
    if result != 0 || length as usize != std::mem::size_of::<libc::pid_t>() || pid <= 0 {
        return Err(OperatorError::new(
            "server_rejected",
            "could not identify the connected Buzz Desktop process",
        ));
    }
    u32::try_from(pid).map_err(|_| {
        OperatorError::new(
            "server_rejected",
            "the connected Buzz Desktop process identifier was invalid",
        )
    })
}

#[cfg(not(target_os = "macos"))]
fn connected_peer_pid(_stream: &StdUnixStream) -> Result<u32, OperatorError> {
    Err(OperatorError::new(
        "server_rejected",
        "the signed Buzz Desktop read service is available only on macOS",
    ))
}

fn validate_connected_server(
    stream: &StdUnixStream,
    socket_path: &Path,
    initial_socket: &fs::Metadata,
) -> Result<u32, OperatorError> {
    let pid = connected_peer_pid(stream)?;
    let current_socket = validate_socket(socket_path)?;
    ensure_server_authentication(
        same_socket_identity(initial_socket, &current_socket),
        production_process_signature_valid(pid as libc::pid_t),
    )?;
    Ok(pid)
}

fn ensure_server_authentication(
    socket_identity_unchanged: bool,
    production_signature_valid: bool,
) -> Result<(), OperatorError> {
    if !socket_identity_unchanged || !production_signature_valid {
        return Err(OperatorError::new(
            "server_rejected",
            "the connected process was not Block's signed Buzz Desktop service",
        ));
    }
    Ok(())
}

fn validate_receipt_binding(
    receipt: &ReadReceipt,
    request_id: &str,
    desktop_pid: u32,
) -> Result<(), OperatorError> {
    if receipt.request_id != request_id || receipt.desktop_pid != desktop_pid {
        return Err(OperatorError::new(
            "receipt_invalid",
            "the Buzz read receipt did not match the request or signed Desktop process",
        ));
    }
    Ok(())
}

/// Expose the bundled credentialless client on the normal local PATH.
///
/// Development bundles deliberately do not create this production command.
/// Existing regular files are preserved; only a symlink in Buzz's own
/// `buzz-read` namespace is refreshed on application boot.
pub fn ensure_client_symlink(exe_parent: &Path) -> Result<(), String> {
    let local_bin = dirs::home_dir()
        .ok_or("cannot resolve home directory")?
        .join(".local")
        .join("bin");
    ensure_client_symlink_at(exe_parent, &local_bin)
}

fn ensure_client_symlink_at(exe_parent: &Path, local_bin: &Path) -> Result<(), String> {
    let bundled = exe_parent.join("buzz-read");
    if !bundled.is_file() || bundled.is_symlink() {
        return Ok(());
    }
    fs::create_dir_all(local_bin).map_err(|error| {
        format!(
            "create Buzz read client directory {}: {error}",
            local_bin.display()
        )
    })?;
    let link = local_bin.join("buzz-read");
    match link.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::remove_file(&link)
                .map_err(|error| format!("remove stale {}: {error}", link.display()))?;
        }
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("stat {}: {error}", link.display())),
    }
    std::os::unix::fs::symlink(&bundled, &link)
        .map_err(|error| format!("symlink {}: {error}", link.display()))
}

async fn handle_connection(
    mut stream: UnixStream,
    app: tauri::AppHandle,
    replay_guard: Arc<ReplayGuard>,
) -> Result<(), OperatorError> {
    validate_peer(&stream)?;

    let raw = timeout(
        SOCKET_IO_TIMEOUT,
        read_frame_async(&mut stream, MAX_REQUEST_BYTES),
    )
    .await
    .map_err(|_| OperatorError::new("request_timeout", "the read request timed out"))??;
    let parsed = serde_json::from_slice::<ReadRequest>(&raw);
    let request_id = parsed
        .as_ref()
        .map(|request| request.request_id.clone())
        .unwrap_or_else(|_| "unknown".to_string());
    let request_expires_at = parsed.as_ref().ok().map(|request| request.expires_at);
    let result = async {
        let request = parsed
            .map_err(|_| OperatorError::new("request_invalid", "the read request was malformed"))?;
        let now = unix_now()?;
        validate_request_at(&request, now)?;
        replay_guard.consume(&request.request_id, request.expires_at, now)?;
        let remaining = duration_until_expiry(request.expires_at)?;
        run_with_expiry_timeout(remaining, execute_read(&app, request)).await
    }
    .await;
    let receipt = result.unwrap_or_else(|error| error_receipt(request_id, &error));
    ensure_receipt_bound(&receipt)?;
    let encoded = serde_json::to_vec(&receipt).map_err(|_| {
        OperatorError::new(
            "receipt_invalid",
            "the Buzz read receipt could not be serialized",
        )
    })?;
    let response_timeout = match request_expires_at {
        Some(expires_at) => duration_until_expiry(expires_at)?.min(SOCKET_IO_TIMEOUT),
        None => SOCKET_IO_TIMEOUT,
    };
    timeout(
        response_timeout,
        write_frame_async(&mut stream, &encoded, MAX_RECEIPT_BYTES),
    )
    .await
    .map_err(|_| OperatorError::new("response_timeout", "the read response timed out"))??;
    Ok(())
}

fn validate_peer(stream: &UnixStream) -> Result<(), OperatorError> {
    let credentials = stream
        .peer_cred()
        .map_err(|_| OperatorError::new("peer_rejected", "could not verify the local caller"))?;
    ensure_peer_uid(credentials.uid(), current_uid())
}

fn ensure_peer_uid(peer_uid: u32, owner_uid: u32) -> Result<(), OperatorError> {
    if peer_uid != owner_uid {
        return Err(OperatorError::new(
            "peer_rejected",
            "the local caller did not match the Buzz Desktop owner",
        ));
    }
    Ok(())
}

async fn execute_read(
    app: &tauri::AppHandle,
    request: ReadRequest,
) -> Result<ReadReceipt, OperatorError> {
    let state = app.state::<AppState>();
    let scope = capture_active_scope(&state).await?;
    if let Some(expected) = request.expected_relay.as_deref() {
        if crate::relay::relay_http_base_url(expected)
            != crate::relay::relay_http_base_url(&scope.relay)
        {
            return Err(OperatorError::new(
                "relay_mismatch",
                "the active Buzz relay did not match the expected relay",
            ));
        }
    }
    if request
        .expected_identity_pubkey
        .as_deref()
        .is_some_and(|expected| !expected.eq_ignore_ascii_case(&scope.identity_pubkey))
    {
        return Err(OperatorError::new(
            "identity_mismatch",
            "the active Buzz identity did not match the expected public key",
        ));
    }

    let mut filter = serde_json::json!({
        "kinds": MESSAGE_KINDS,
        "since": request.since,
        "until": request.until,
        "limit": request.limit.saturating_add(1),
    });
    if let Some(channel) = request.channel.as_deref() {
        filter["#h"] = serde_json::json!([channel]);
    }
    if let Some(search) = request.search.as_deref() {
        filter["search"] = serde_json::json!(search.trim());
    }

    let api_base = crate::relay::relay_http_base_url(&scope.relay);
    let mut events = query_verified(
        &state,
        &api_base,
        &[filter],
        &scope.keys,
        request.expires_at,
    )
    .await?;
    assert_active_scope_unchanged(&state, &scope).await?;
    if events.len() > request.limit.saturating_add(1) as usize {
        return Err(OperatorError::new(
            "response_oversize",
            "the Buzz relay returned more events than requested",
        ));
    }
    events.retain(|event| event_matches_request(event, &request));
    events.sort_by(|left, right| {
        right
            .created_at
            .as_secs()
            .cmp(&left.created_at.as_secs())
            .then_with(|| left.id.to_hex().cmp(&right.id.to_hex()))
    });
    let mut seen = HashSet::new();
    events.retain(|event| seen.insert(event.id.to_hex()));
    let truncated = events.len() > request.limit as usize;
    events.truncate(request.limit as usize);

    let author_names =
        fetch_author_names(&state, &api_base, &scope.keys, &events, request.expires_at)
            .await
            .unwrap_or_default();
    assert_active_scope_unchanged(&state, &scope).await?;
    ensure_request_not_expired_at(request.expires_at, unix_now()?)?;
    let projected = events
        .iter()
        .map(|event| project_event(event, request.excerpt_chars, &author_names))
        .collect::<Vec<_>>();
    let receipt = ReadReceipt {
        schema_version: SCHEMA_VERSION,
        request_id: request.request_id,
        status: "ok".to_string(),
        operation: "messages".to_string(),
        generated_at: unix_now()?,
        desktop_pid: std::process::id(),
        relay_host: ALLOWED_RELAY_HOST.to_string(),
        identity_pubkey: scope.identity_pubkey,
        requested_limit: request.limit,
        returned: projected.len(),
        truncated,
        events: projected,
        error_code: None,
        message: None,
    };
    ensure_receipt_bound(&receipt)?;
    Ok(receipt)
}

async fn capture_active_scope(state: &AppState) -> Result<ActiveScope, OperatorError> {
    let _workspace_guard = state.workspace_apply_lock.lock().await;
    let workspace_generation = state.workspace_apply_generation.load(Ordering::Acquire);
    let relay = crate::relay::relay_ws_url_with_override(state);
    validate_relay(&relay)?;
    let identity = capture_production_identity(state)?;
    let identity_pubkey = identity.keys.public_key().to_hex();
    Ok(ActiveScope {
        relay,
        keys: identity.keys,
        identity_pubkey,
        identity_generation: identity.generation,
        workspace_generation,
    })
}

async fn assert_active_scope_unchanged(
    state: &AppState,
    initial: &ActiveScope,
) -> Result<(), OperatorError> {
    let _workspace_guard = state.workspace_apply_lock.lock().await;
    let current_generation = state.workspace_apply_generation.load(Ordering::Acquire);
    let current_relay = crate::relay::relay_ws_url_with_override(state);
    let current_identity = capture_production_identity(state)?;
    let current_pubkey = current_identity.keys.public_key().to_hex();
    ensure_scope_values_unchanged(
        ScopeFingerprint {
            workspace_generation: initial.workspace_generation,
            identity_generation: initial.identity_generation,
            relay: &initial.relay,
            pubkey: &initial.identity_pubkey,
        },
        ScopeFingerprint {
            workspace_generation: current_generation,
            identity_generation: current_identity.generation,
            relay: &current_relay,
            pubkey: &current_pubkey,
        },
    )
}

fn capture_production_keys(state: &AppState) -> Result<Keys, OperatorError> {
    capture_production_identity(state).map(|identity| identity.keys)
}

fn capture_production_identity(state: &AppState) -> Result<ProductionIdentity, OperatorError> {
    capture_production_identity_with_signature(state, production_code_signature_valid())
}

fn capture_production_identity_with_signature(
    state: &AppState,
    code_signature_valid: bool,
) -> Result<ProductionIdentity, OperatorError> {
    // Identity writers change both the keys and their storage classification
    // while holding this mutex. Read the classification only after acquiring
    // the same mutex so a same-key keyring-to-file import cannot slip between
    // an owner check and the cloned signing keys.
    let keys = state.keys.lock().map_err(|_| {
        OperatorError::new(
            "identity_unavailable",
            "the active Buzz Desktop identity was unavailable",
        )
    })?;
    let recovery_mode =
        state.identity_lost.load(Ordering::Acquire) || state.keyring_locked.load(Ordering::Acquire);
    if recovery_mode
        || !production_credential_owner_allowed(
            PRODUCTION_BUNDLE_IDENTIFIER,
            state.identity_storage(),
            code_signature_valid,
        )
    {
        return Err(OperatorError::new(
            "identity_unavailable",
            "the active identity is no longer owned by Block's signed production app keyring",
        ));
    }
    Ok(ProductionIdentity {
        keys: keys.clone(),
        generation: state.identity_generation.load(Ordering::Acquire),
    })
}

fn ensure_scope_values_unchanged(
    initial: ScopeFingerprint<'_>,
    current: ScopeFingerprint<'_>,
) -> Result<(), OperatorError> {
    if initial.workspace_generation != current.workspace_generation
        || initial.identity_generation != current.identity_generation
        || initial.relay != current.relay
        || initial.pubkey != current.pubkey
    {
        return Err(OperatorError::new(
            "active_scope_changed",
            "the active Buzz workspace changed during the read",
        ));
    }
    Ok(())
}

async fn query_verified(
    state: &AppState,
    api_base: &str,
    filters: &[serde_json::Value],
    keys: &Keys,
    expires_at: i64,
) -> Result<Vec<Event>, OperatorError> {
    crate::relay_admission::wait_for_rate_limit().await;
    ensure_request_not_expired_at(expires_at, unix_now()?)?;
    let url = format!("{}/query", api_base.trim_end_matches('/'));
    let body = serde_json::to_vec(filters).map_err(|_| {
        OperatorError::new(
            "request_invalid",
            "the Buzz relay filter could not be serialized",
        )
    })?;
    let authorization =
        crate::relay::build_nip98_auth_header_for_keys(keys, &Method::POST, &url, &body).map_err(
            |_| {
                OperatorError::new(
                    "identity_unavailable",
                    "Buzz Desktop could not authenticate the read",
                )
            },
        )?;
    let response = state
        .media_fetch_client
        .post(url)
        .header("Authorization", authorization)
        .header("Content-Type", "application/json")
        .timeout(Duration::from_secs(30))
        .body(body)
        .send()
        .await
        .map_err(|_| {
            OperatorError::new(
                "relay_unavailable",
                "the Buzz relay could not complete the authenticated read",
            )
        })?;
    if !response.status().is_success() {
        return Err(OperatorError::new(
            "relay_rejected",
            "the Buzz relay rejected the authenticated read",
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RELAY_RESPONSE_BYTES as u64)
    {
        return Err(OperatorError::new(
            "response_oversize",
            "the Buzz relay response exceeded the read bound",
        ));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            OperatorError::new(
                "relay_unavailable",
                "the Buzz relay response was interrupted",
            )
        })?;
        if bytes.len().saturating_add(chunk.len()) > MAX_RELAY_RESPONSE_BYTES {
            return Err(OperatorError::new(
                "response_oversize",
                "the Buzz relay response exceeded the read bound",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let events: Vec<Event> = serde_json::from_slice(&bytes).map_err(|_| {
        OperatorError::new("response_invalid", "the Buzz relay response was malformed")
    })?;
    verify_event_set(&events)?;
    ensure_request_not_expired_at(expires_at, unix_now()?)?;
    Ok(events)
}

fn verify_event_set(events: &[Event]) -> Result<(), OperatorError> {
    for event in events {
        event.verify().map_err(|_| {
            OperatorError::new(
                "response_unverified",
                "the Buzz relay returned an event that failed verification",
            )
        })?;
    }
    Ok(())
}

async fn fetch_author_names(
    state: &AppState,
    api_base: &str,
    keys: &Keys,
    events: &[Event],
    expires_at: i64,
) -> Result<HashMap<String, String>, OperatorError> {
    let authors = events
        .iter()
        .map(|event| event.pubkey.to_hex())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if authors.is_empty() {
        return Ok(HashMap::new());
    }
    let filter = serde_json::json!({
        "kinds": [0],
        "authors": authors,
        "limit": authors.len().min(MAX_RESULTS as usize),
    });
    let profiles = query_verified(state, api_base, &[filter], keys, expires_at).await?;
    if profiles.len() > MAX_RESULTS as usize {
        return Err(OperatorError::new(
            "response_oversize",
            "the Buzz relay returned too many author profiles",
        ));
    }
    let mut names = HashMap::new();
    for profile in profiles {
        let pubkey = profile.pubkey.to_hex();
        if names.contains_key(&pubkey) {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&profile.content) else {
            continue;
        };
        let name = value
            .get("display_name")
            .or_else(|| value.get("name"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty() && name.chars().count() <= 80)
            .map(redact_sensitive_text);
        if let Some(name) = name {
            names.insert(pubkey, name);
        }
    }
    Ok(names)
}

fn event_matches_request(event: &Event, request: &ReadRequest) -> bool {
    let kind = event.kind.as_u16() as u32;
    let created_at = event.created_at.as_secs() as i64;
    MESSAGE_KINDS.contains(&kind)
        && created_at >= request.since
        && created_at <= request.until
        && request
            .channel
            .as_deref()
            .is_none_or(|expected| event_channel(event).as_deref() == Some(expected))
        && request.search.as_deref().is_none_or(|search| {
            event
                .content
                .to_lowercase()
                .contains(&search.trim().to_lowercase())
        })
}

fn project_event(
    event: &Event,
    excerpt_chars: u32,
    author_names: &HashMap<String, String>,
) -> ReceiptEvent {
    let author_pubkey = event.pubkey.to_hex();
    let excerpt = (excerpt_chars > 0).then(|| bounded_excerpt(&event.content, excerpt_chars));
    ReceiptEvent {
        id: event.id.to_hex(),
        author_name: author_names.get(&author_pubkey).cloned(),
        author_pubkey,
        kind: event.kind.as_u16() as u32,
        created_at: event.created_at.as_secs() as i64,
        channel: event_channel(event),
        excerpt,
    }
}

fn event_channel(event: &Event) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        if values.first().map(String::as_str) != Some("h") {
            return None;
        }
        let channel = values.get(1)?;
        uuid::Uuid::parse_str(channel).ok().map(|_| channel.clone())
    })
}

fn bounded_excerpt(content: &str, max_chars: u32) -> String {
    let normalized = content
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let redacted = redact_sensitive_text(&normalized);
    let maximum = max_chars as usize;
    if redacted.chars().count() <= maximum {
        return redacted;
    }
    if maximum == 0 {
        return String::new();
    }
    let mut excerpt = redacted.chars().take(maximum - 1).collect::<String>();
    excerpt.push('…');
    excerpt
}

fn redact_sensitive_text(input: &str) -> String {
    static PATTERNS: std::sync::OnceLock<Vec<(Regex, &'static str)>> = std::sync::OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        vec![
            (
                Regex::new(r"(?i)nsec1[023456789acdefghjklmnpqrstuvwxyz]{16,}")
                    .expect("static nsec regex"),
                "[REDACTED_NSEC]",
            ),
            (
                Regex::new(r"(?i)(?:sk-|ghp_|github_pat_|xox[baprs]-)[A-Za-z0-9_-]{10,}")
                    .expect("static token regex"),
                "[REDACTED_TOKEN]",
            ),
            (
                Regex::new(r"(?i)(bearer\s+)[^\s,;]+")
                    .expect("static bearer regex"),
                "$1[REDACTED]",
            ),
            (
                Regex::new(r"(?i)((?:buzz_private_key|buzz_auth_tag|authorization|password|api[_ -]?key|token|secret)\s*[:=]\s*)[^\s,;]+")
                    .expect("static assignment regex"),
                "$1[REDACTED]",
            ),
        ]
    });
    patterns
        .iter()
        .fold(input.to_string(), |current, (regex, replacement)| {
            regex.replace_all(&current, *replacement).to_string()
        })
}

fn error_receipt(request_id: String, error: &OperatorError) -> ReadReceipt {
    ReadReceipt {
        schema_version: SCHEMA_VERSION,
        request_id,
        status: "error".to_string(),
        operation: "messages".to_string(),
        generated_at: unix_now().unwrap_or(0),
        desktop_pid: std::process::id(),
        relay_host: String::new(),
        identity_pubkey: String::new(),
        requested_limit: 0,
        returned: 0,
        truncated: false,
        events: Vec::new(),
        error_code: Some(error.code.to_string()),
        message: Some(error.message.to_string()),
    }
}

fn ensure_receipt_bound(receipt: &ReadReceipt) -> Result<(), OperatorError> {
    let bytes = serde_json::to_vec(receipt).map_err(|_| {
        OperatorError::new(
            "receipt_invalid",
            "the Buzz read receipt could not be serialized",
        )
    })?;
    if bytes.len() > MAX_RECEIPT_BYTES {
        return Err(OperatorError::new(
            "receipt_oversize",
            "the Buzz read receipt exceeded its output bound",
        ));
    }
    Ok(())
}

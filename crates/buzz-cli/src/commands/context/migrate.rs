use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use serde::Serialize;

use super::profile::{
    discover_legacy_optional_roles, legacy_profile, ProfileEnvironment, ProfileFile,
};
use crate::error::CliError;

#[derive(Debug, Clone, Serialize)]
pub struct MigrationReport {
    pub profile: String,
    pub mode: &'static str,
    pub status: &'static str,
    pub legacy_root: String,
    pub profile_path: String,
    pub actions: Vec<MigrationAction>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MigrationAction {
    pub operation: &'static str,
    pub source: Option<String>,
    pub destination: Option<String>,
    pub detail: String,
}

pub struct MigrationRequest<'a> {
    pub profile: &'a str,
    pub legacy_root: Option<&'a Path>,
    pub local_relay: &'a str,
    pub rendezvous: Option<&'a str>,
    pub default_context: Option<&'a str>,
    pub apply: bool,
}

pub fn migrate(
    request: MigrationRequest<'_>,
    environment: &ProfileEnvironment,
) -> Result<MigrationReport, CliError> {
    super::profile::validate_profile_name(request.profile)?;
    let legacy_root = request
        .legacy_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| environment.home.join(".buzz-local"));
    let profile_path = environment
        .config_home
        .join("buzz/profiles")
        .join(format!("{}.toml", request.profile));
    let profile_dir = profile_path.parent().ok_or_else(|| {
        CliError::Other(format!(
            "profile path has no parent: {}",
            profile_path.display()
        ))
    })?;
    let target_data = environment
        .data_home
        .join("buzz-local-relay")
        .join(request.profile);

    if !path_has_entries(&legacy_root)? {
        return Err(CliError::NotFound(format!(
            "legacy context layout is absent or empty at {}",
            legacy_root.display()
        )));
    }

    let mut profile = legacy_profile(
        &legacy_root,
        request.local_relay.to_string(),
        request.rendezvous.map(str::to_string),
        request.default_context.map(str::to_string),
    );
    profile.node.label = environment
        .variables
        .get("BUZZ_CTX_NODE")
        .cloned()
        .or_else(|| Some(request.profile.to_string()));
    discover_legacy_optional_roles(&legacy_root, &environment.variables, &mut profile);
    let serialized = toml::to_string_pretty(&profile)
        .map_err(|error| CliError::Other(format!("profile serialization failed: {error}")))?;

    if profile_path.exists() {
        let existing = std::fs::read_to_string(&profile_path).map_err(|error| {
            CliError::Other(format!(
                "could not read {}: {error}",
                profile_path.display()
            ))
        })?;
        let parsed: ProfileFile = toml::from_str(&existing).map_err(|error| {
            CliError::Conflict(format!(
                "existing profile {} is invalid: {error}",
                profile_path.display()
            ))
        })?;
        if parsed.data_root == legacy_root {
            return Ok(MigrationReport {
                profile: request.profile.into(),
                mode: if request.apply { "apply" } else { "dry_run" },
                status: "already_applied",
                legacy_root: legacy_root.display().to_string(),
                profile_path: profile_path.display().to_string(),
                actions: vec![MigrationAction {
                    operation: "none",
                    source: Some(legacy_root.display().to_string()),
                    destination: Some(profile_path.display().to_string()),
                    detail: "existing profile already references the legacy layout".into(),
                }],
            });
        }
        return Err(CliError::Conflict(format!(
            "profile {} already exists and points at {}; refusing to mix it with legacy state {}",
            profile_path.display(),
            parsed.data_root.display(),
            legacy_root.display()
        )));
    }
    if path_has_entries(&target_data)? {
        return Err(CliError::Conflict(format!(
            "new profile data already exists at {} while legacy state exists at {}; refusing ambiguous mixed state",
            target_data.display(),
            legacy_root.display()
        )));
    }

    let mut actions = vec![
        MigrationAction {
            operation: "create_directory",
            source: None,
            destination: Some(profile_dir.display().to_string()),
            detail: "create the XDG profile directory with owner-only permissions".into(),
        },
        MigrationAction {
            operation: "write_profile",
            source: None,
            destination: Some(profile_path.display().to_string()),
            detail: "atomically install a profile containing references, never secret bytes".into(),
        },
        MigrationAction {
            operation: "reference_state",
            source: Some(legacy_root.display().to_string()),
            destination: Some(profile_path.display().to_string()),
            detail: "keep durable relay state in place for rollback; no journal, cursor, or artifact bytes move".into(),
        },
    ];
    for (role, identity) in profile.identities.named() {
        let Some(identity) = identity else {
            continue;
        };
        actions.push(MigrationAction {
            operation: "reference_credential",
            source: Some(identity.reference.clone()),
            destination: Some(format!("identities.{role}")),
            detail: "store only the credential-provider reference; key material is neither read nor copied".into(),
        });
    }

    if request.apply {
        install_profile(profile_dir, &profile_path, serialized.as_bytes())?;
    }
    Ok(MigrationReport {
        profile: request.profile.into(),
        mode: if request.apply { "apply" } else { "dry_run" },
        status: if request.apply { "applied" } else { "planned" },
        legacy_root: legacy_root.display().to_string(),
        profile_path: profile_path.display().to_string(),
        actions,
    })
}

fn path_has_entries(path: &Path) -> Result<bool, CliError> {
    if !path.exists() {
        return Ok(false);
    }
    if !path.is_dir() {
        return Ok(true);
    }
    let mut entries = std::fs::read_dir(path).map_err(|error| {
        CliError::Other(format!("could not inspect {}: {error}", path.display()))
    })?;
    Ok(entries
        .next()
        .transpose()
        .map_err(|error| CliError::Other(format!("could not inspect {}: {error}", path.display())))?
        .is_some())
}

fn install_profile(directory: &Path, destination: &Path, bytes: &[u8]) -> Result<(), CliError> {
    std::fs::create_dir_all(directory).map_err(|error| {
        CliError::Other(format!(
            "could not create profile directory {}: {error}",
            directory.display()
        ))
    })?;
    set_owner_only(directory, true)?;
    let temporary = destination.with_extension("toml.tmp");
    if temporary.exists() {
        let existing = std::fs::read(&temporary).map_err(|error| {
            CliError::Other(format!(
                "could not inspect interrupted migration {}: {error}",
                temporary.display()
            ))
        })?;
        if existing != bytes {
            return Err(CliError::Conflict(format!(
                "interrupted migration file {} differs from the current plan",
                temporary.display()
            )));
        }
    } else {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| {
                CliError::Other(format!(
                    "could not create migration file {}: {error}",
                    temporary.display()
                ))
            })?;
        set_owner_only(&temporary, false)?;
        file.write_all(bytes).map_err(|error| {
            CliError::Other(format!(
                "could not write migration file {}: {error}",
                temporary.display()
            ))
        })?;
        file.sync_all().map_err(|error| {
            CliError::Other(format!(
                "could not sync migration file {}: {error}",
                temporary.display()
            ))
        })?;
    }
    std::fs::rename(&temporary, destination).map_err(|error| {
        CliError::Other(format!(
            "could not atomically install profile {}: {error}",
            destination.display()
        ))
    })?;
    Ok(())
}

#[cfg(unix)]
fn set_owner_only(path: &Path, directory: bool) -> Result<(), CliError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if directory { 0o700 } else { 0o600 };
    let permissions = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, permissions).map_err(|error| {
        CliError::Other(format!(
            "could not set owner-only permissions on {}: {error}",
            path.display()
        ))
    })
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path, _directory: bool) -> Result<(), CliError> {
    Ok(())
}

pub fn print_report(report: &MigrationReport, json: bool) -> Result<(), CliError> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(report).map_err(|error| {
                CliError::Other(format!("migration serialization failed: {error}"))
            })?
        );
        return Ok(());
    }
    println!(
        "buzz context migrate — {} ({})",
        report.profile, report.mode
    );
    println!("  status   {}", report.status);
    println!("  legacy   {}", report.legacy_root);
    println!("  profile  {}", report.profile_path);
    for action in &report.actions {
        let source = action.source.as_deref().unwrap_or("-");
        let destination = action.destination.as_deref().unwrap_or("-");
        println!(
            "  {:<20} {} -> {} — {}",
            action.operation, source, destination, action.detail
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::TempDir;

    use super::*;

    fn environment(root: &TempDir) -> ProfileEnvironment {
        let home = root.path().join("home");
        ProfileEnvironment {
            config_home: home.join(".config"),
            data_home: home.join(".local/share"),
            ctx_home_override: None,
            variables: BTreeMap::new(),
            home,
        }
    }

    #[test]
    fn legacy_migration_is_dry_run_first_and_never_copies_keys() {
        let root = TempDir::new().expect("tempdir");
        let env = environment(&root);
        let legacy = env.home.join(".buzz-local");
        std::fs::create_dir_all(&legacy).expect("legacy");
        std::fs::write(legacy.join("node.key"), "do-not-copy").expect("key");
        std::fs::write(legacy.join("sovereign.ndjson"), "{}\n").expect("journal");

        let report = migrate(
            MigrationRequest {
                profile: "default",
                legacy_root: None,
                local_relay: "http://127.0.0.1:7777",
                rendezvous: Some("https://relay.example"),
                default_context: None,
                apply: false,
            },
            &env,
        )
        .expect("migration plans");
        assert_eq!(report.status, "planned");
        assert!(!Path::new(&report.profile_path).exists());
        assert!(report
            .actions
            .iter()
            .any(|action| action.operation == "reference_credential"));
        let encoded = serde_json::to_string(&report).expect("report serializes");
        assert!(!encoded.contains("do-not-copy"));
    }

    #[test]
    fn migration_refuses_conflicting_old_and_new_state() {
        let root = TempDir::new().expect("tempdir");
        let env = environment(&root);
        let legacy = env.home.join(".buzz-local");
        let target = env.data_home.join("buzz-local-relay/default");
        std::fs::create_dir_all(&legacy).expect("legacy");
        std::fs::create_dir_all(&target).expect("target");
        std::fs::write(legacy.join("sovereign.ndjson"), "legacy").expect("legacy journal");
        std::fs::write(target.join("sovereign.ndjson"), "new").expect("new journal");
        let error = migrate(
            MigrationRequest {
                profile: "default",
                legacy_root: None,
                local_relay: "http://127.0.0.1:7777",
                rendezvous: None,
                default_context: None,
                apply: false,
            },
            &env,
        )
        .expect_err("mixed state must fail");
        assert!(matches!(error, CliError::Conflict(_)));
    }

    #[test]
    fn applied_migration_is_restart_safe_and_references_state_in_place() {
        let root = TempDir::new().expect("tempdir");
        let env = environment(&root);
        let legacy = env.home.join(".buzz-local");
        std::fs::create_dir_all(&legacy).expect("legacy");
        std::fs::write(legacy.join("sovereign.ndjson"), "legacy").expect("journal");
        let request = || MigrationRequest {
            profile: "enterprise",
            legacy_root: Some(&legacy),
            local_relay: "http://127.0.0.1:7777",
            rendezvous: Some("https://relay.example"),
            default_context: Some("context-id"),
            apply: true,
        };
        let first = migrate(request(), &env).expect("first migration");
        assert_eq!(first.status, "applied");
        let second = migrate(request(), &env).expect("second migration");
        assert_eq!(second.status, "already_applied");
        let profile: ProfileFile =
            toml::from_str(&std::fs::read_to_string(&first.profile_path).expect("profile reads"))
                .expect("profile parses");
        assert_eq!(profile.data_root, legacy);
    }
}

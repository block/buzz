use super::*;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

use crate::plugin_host::{PluginError, PluginHost, PluginHostConfig};

fn target_triple() -> String {
    env!("BUZZ_PLUGIN_TARGET_TRIPLE").to_string()
}

/// Writes a minimal, otherwise-valid package directory: a manifest whose one
/// `runtime.targets` entry matches this build's target triple, and an
/// executable script at that path with real digest/length.
fn write_package(directory: &Path, plugin_id: &str, executable_body: &[u8]) -> (u64, String) {
    let triple = target_triple();
    let binary_directory = directory.join("bin");
    fs::create_dir_all(&binary_directory).expect("create bin directory");
    let executable_path = binary_directory.join(&triple);
    fs::write(&executable_path, executable_body).expect("write executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&executable_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable_path, permissions).unwrap();
    }
    let bytes = executable_body.len() as u64;
    let sha256 = hex::encode(Sha256::digest(executable_body));

    let manifest = json!({
        "packageFormatVersion": "0.1.0-alpha",
        "contractVersion": "0.1.0-alpha",
        "id": plugin_id,
        "name": "Package Test Plugin",
        "version": "0.1.0",
        "publisher": "Buzz Tests",
        "license": "Apache-2.0",
        "runtime": {
            "type": "stdio",
            "targets": {
                triple.clone(): { "path": format!("bin/{triple}"), "sha256": sha256, "bytes": bytes }
            }
        },
        "grants": ["browser.browse"],
        "contributions": [{ "kind": "browser", "id": "web", "title": "Web" }],
    });
    fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .expect("write manifest");
    (bytes, sha256)
}

fn test_config(root: &Path) -> PluginHostConfig {
    PluginHostConfig {
        root: root.to_path_buf(),
        resolve_deadline: Duration::from_millis(200),
        discover_deadline: Duration::from_secs(2),
    }
}

#[test]
fn stages_a_valid_package_with_matching_digest() {
    let package = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    let (_bytes, sha256) = write_package(package.path(), "dev.example.stage", b"#!/bin/sh\n");

    let staged =
        stage(staging.path(), package.path(), staging.path()).expect("stage a valid package");
    assert_eq!(staged.executable_sha256, sha256);
}

#[test]
fn rejects_a_package_over_the_file_count_limit() {
    let package = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    write_package(package.path(), "dev.example.toomanyfiles", b"#!/bin/sh\n");
    for index in 0..20 {
        fs::write(package.path().join(format!("extra-{index}.txt")), b"x").unwrap();
    }

    let result = stage(staging.path(), package.path(), staging.path());
    assert!(matches!(result, Err(InstallReject::InvalidPackage(_))));
}

#[test]
fn rejects_a_package_over_the_total_size_limit() {
    let package = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    write_package(package.path(), "dev.example.toobig", b"#!/bin/sh\n");
    let oversized = vec![0_u8; 65 * 1024 * 1024];
    fs::write(package.path().join("payload.bin"), &oversized).unwrap();

    let result = stage(staging.path(), package.path(), staging.path());
    assert!(matches!(result, Err(InstallReject::InvalidPackage(_))));
}

#[cfg(unix)]
#[test]
fn rejects_a_symlink_inside_the_package() {
    let package = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    write_package(package.path(), "dev.example.symlink", b"#!/bin/sh\n");
    let real_target = package.path().join("real.txt");
    fs::write(&real_target, b"data").unwrap();
    std::os::unix::fs::symlink(&real_target, package.path().join("link.txt")).unwrap();

    let result = stage(staging.path(), package.path(), staging.path());
    assert!(matches!(result, Err(InstallReject::InvalidPackage(_))));
}

#[cfg(unix)]
#[test]
fn rejects_a_non_regular_file_inside_the_package() {
    let package = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    write_package(package.path(), "dev.example.fifo", b"#!/bin/sh\n");
    let fifo_path = package.path().join("pipe");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo_path)
        .status()
        .unwrap();
    assert!(status.success());

    let result = stage(staging.path(), package.path(), staging.path());
    assert!(matches!(result, Err(InstallReject::InvalidPackage(_))));
}

#[test]
fn rejects_a_digest_that_does_not_match_the_extracted_executable() {
    let package = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    write_package(package.path(), "dev.example.digest", b"#!/bin/sh\n");
    // Corrupt the manifest's declared digest without touching bytes/path.
    let manifest_path = package.path().join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let triple = target_triple();
    manifest["runtime"]["targets"][&triple]["sha256"] = json!("f".repeat(64));
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let result = stage(staging.path(), package.path(), staging.path());
    assert_eq!(result, Err(InstallReject::DigestMismatch));
}

#[test]
fn rejects_a_manifest_with_no_entry_for_the_running_target_triple() {
    let package = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    write_package(package.path(), "dev.example.notarget", b"#!/bin/sh\n");
    let manifest_path = package.path().join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let triple = target_triple();
    let target = manifest["runtime"]["targets"][&triple].clone();
    manifest["runtime"]["targets"] = json!({ "unknown-triple-xyz": target });
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let result = stage(staging.path(), package.path(), staging.path());
    assert_eq!(result, Err(InstallReject::UnknownTarget));
}

#[tokio::test]
async fn rejects_installing_an_already_installed_plugin_id() {
    let package = tempfile::tempdir().unwrap();
    write_package(package.path(), "dev.example.duplicate", b"#!/bin/sh\n");
    let host_root = tempfile::tempdir().unwrap();
    let host = PluginHost::new(test_config(host_root.path()));

    let first = host.install(package.path());
    assert!(first.is_ok(), "first install: {first:?}");

    let second = host.install(package.path());
    assert!(
        matches!(
            second,
            Err(PluginError::Install(InstallReject::AlreadyInstalled))
        ),
        "second install of the same id: {second:?}"
    );
    let listed = host.list().expect("list after duplicate rejection");
    assert_eq!(listed.len(), 1, "duplicate rejection leaves one record");
}

#[tokio::test]
async fn rejects_installing_over_a_disabled_installation() {
    let package = tempfile::tempdir().unwrap();
    write_package(package.path(), "dev.example.disableddup", b"#!/bin/sh\n");
    let host_root = tempfile::tempdir().unwrap();
    let host = PluginHost::new(test_config(host_root.path()));

    host.install(package.path()).expect("install once");
    host.set_enabled("dev.example.disableddup", false)
        .expect("disable installed plugin");

    let second = host.install(package.path());
    assert!(
        matches!(
            second,
            Err(PluginError::Install(InstallReject::AlreadyInstalled))
        ),
        "install over a disabled id: {second:?}"
    );
    let listed = host.list().expect("list after rejection");
    assert_eq!(listed.len(), 1);
    assert!(
        !listed[0].enabled,
        "the existing disabled record is unchanged"
    );
}

#[tokio::test]
async fn disable_keeps_the_package_and_uninstall_removes_it() {
    let package = tempfile::tempdir().unwrap();
    write_package(package.path(), "dev.example.lifecycle", b"#!/bin/sh\n");
    let host_root = tempfile::tempdir().unwrap();
    let host = PluginHost::new(test_config(host_root.path()));
    host.install(package.path()).expect("install fixture");

    host.set_enabled("dev.example.lifecycle", false)
        .expect("disable");
    let disabled = host.list().expect("list disabled");
    assert_eq!(disabled.len(), 1);
    assert!(!disabled[0].enabled);

    host.set_enabled("dev.example.lifecycle", true)
        .expect("re-enable");
    let enabled = host.list().expect("list enabled");
    assert!(enabled[0].enabled);

    host.uninstall("dev.example.lifecycle").expect("uninstall");
    let remaining = host.list().expect("list after uninstall");
    assert!(remaining.is_empty());
}

#[test]
fn accepts_a_file_at_exactly_the_directory_nesting_limit() {
    // Nesting depth is a property of directories, not of the files inside
    // them: a file directly inside the 16th nested directory (the contract
    // limit) must still be accepted.
    let package = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    write_package(package.path(), "dev.example.exactdepth", b"#!/bin/sh\n");
    let mut deep = package.path().to_path_buf();
    for index in 0..16 {
        deep = deep.join(format!("d{index}"));
    }
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("f.txt"), b"x").unwrap();

    let result = stage(staging.path(), package.path(), staging.path());
    assert!(
        result.is_ok(),
        "a file 16 directories deep is exactly at the limit: {result:?}"
    );
}

#[test]
fn rejects_a_directory_one_over_the_nesting_limit() {
    let package = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    write_package(package.path(), "dev.example.toodeep", b"#!/bin/sh\n");
    let mut deep = package.path().to_path_buf();
    for index in 0..17 {
        deep = deep.join(format!("d{index}"));
    }
    fs::create_dir_all(&deep).unwrap();

    let result = stage(staging.path(), package.path(), staging.path());
    assert!(matches!(result, Err(InstallReject::InvalidPackage(_))));
}

#[test]
fn rejects_a_package_over_the_total_entry_limit() {
    let package = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    write_package(package.path(), "dev.example.toomanyentries", b"#!/bin/sh\n");
    for index in 0..260 {
        fs::create_dir(package.path().join(format!("dir-{index}"))).unwrap();
    }

    let result = stage(staging.path(), package.path(), staging.path());
    assert!(matches!(result, Err(InstallReject::InvalidPackage(_))));
}

#[test]
fn failed_stage_removes_the_staging_directory() {
    let package = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    write_package(package.path(), "dev.example.cleanup", b"#!/bin/sh\n");
    let manifest_path = package.path().join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["license"] = json!(""); // fails manifest validation after the copy succeeds
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let result = stage(staging.path(), package.path(), staging.path());
    assert!(result.is_err());
    let mut remaining = fs::read_dir(staging.path()).unwrap();
    assert!(
        remaining.next().is_none(),
        "a failed stage must not leave a staging directory behind"
    );
}

#[cfg(unix)]
#[test]
fn rejects_a_symlinked_package_root() {
    let real = tempfile::tempdir().unwrap();
    write_package(real.path(), "dev.example.symlinkroot", b"#!/bin/sh\n");
    let holder = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    let link_path = holder.path().join("pkg-link");
    std::os::unix::fs::symlink(real.path(), &link_path).unwrap();

    let result = stage(staging.path(), &link_path, staging.path());
    assert!(matches!(result, Err(InstallReject::InvalidPackage(_))));
}

#[cfg(unix)]
#[test]
fn refuses_to_stage_into_a_symlinked_staging_root() {
    let package = tempfile::tempdir().unwrap();
    write_package(package.path(), "dev.example.stagingsymlink", b"#!/bin/sh\n");
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("sentinel.txt"), b"do not touch").unwrap();
    let staging_holder = tempfile::tempdir().unwrap();
    let staging_link = staging_holder.path().join("staging-link");
    std::os::unix::fs::symlink(external.path(), &staging_link).unwrap();

    let result = stage(&staging_link, package.path(), &staging_link);
    assert!(matches!(result, Err(InstallReject::InvalidPackage(_))));
    assert!(
        external.path().join("sentinel.txt").exists(),
        "a symlinked staging root must not redirect writes into the target directory"
    );
}

#[cfg(unix)]
#[test]
fn reconcile_refuses_to_touch_children_of_a_symlinked_root() {
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("keep.txt"), b"external data").unwrap();
    let trusted_base = tempfile::tempdir().unwrap();
    let symlinked_parent = trusted_base.path().join("plugins-link");
    std::os::unix::fs::symlink(external.path(), &symlinked_parent).unwrap();
    let staging_root = trusted_base.path().join("staging");

    reconcile(
        trusted_base.path(),
        &symlinked_parent,
        &HashSet::new(),
        &staging_root,
    )
    .expect("reconcile does not error on a symlinked parent");

    assert!(
        external.path().join("keep.txt").exists(),
        "external data behind a symlinked owned root must survive reconcile"
    );
}

#[cfg(unix)]
#[test]
fn reconcile_refuses_a_symlinked_ancestor_inside_owned_storage() {
    // Unlike the test above, `parent` (`id_root`) is not itself the symlink —
    // its own leaf, resolved through the symlinked `plugins/` ancestor, is an
    // ordinary directory. A leaf-only symlink check would miss this and
    // delete the external directory's unreferenced children.
    let trusted_base = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let external_id_dir = external.path().join("some-id");
    fs::create_dir(&external_id_dir).unwrap();
    fs::write(external_id_dir.join("keep.txt"), b"external data").unwrap();
    std::os::unix::fs::symlink(external.path(), trusted_base.path().join("plugins")).unwrap();
    let id_root = trusted_base.path().join("plugins").join("some-id");
    let staging_root = trusted_base.path().join("staging");

    reconcile(
        trusted_base.path(),
        &id_root,
        &HashSet::new(),
        &staging_root,
    )
    .expect("reconcile does not error when an ancestor is symlinked");

    assert!(
        external_id_dir.join("keep.txt").exists(),
        "external data reached through a symlinked ancestor inside owned storage must survive reconcile"
    );
}

#[cfg(unix)]
#[test]
fn ensure_owned_directory_refuses_a_symlinked_ancestor_inside_owned_storage() {
    let trusted_base = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(external.path(), trusted_base.path().join("plugins")).unwrap();
    let leaf = trusted_base.path().join("plugins").join("some-id");

    let result = ensure_owned_directory(trusted_base.path(), &leaf);

    assert!(
        result.is_err(),
        "a symlinked ancestor inside owned storage must be refused even though the leaf itself, \
         resolved through it, is an ordinary path"
    );
}

#[cfg(unix)]
#[test]
fn remove_owned_directory_refuses_a_symlinked_ancestor_inside_owned_storage() {
    // Mirrors `reconcile`'s ancestry gap on the direct removal path used by
    // uninstall: `plugins/<id>` is not itself a symlink, but resolves through
    // a symlinked `plugins/` into external data.
    let trusted_base = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let external_id_dir = external.path().join("some-id");
    fs::create_dir(&external_id_dir).unwrap();
    fs::write(external_id_dir.join("keep.txt"), b"external data").unwrap();
    std::os::unix::fs::symlink(external.path(), trusted_base.path().join("plugins")).unwrap();
    let id_root = trusted_base.path().join("plugins").join("some-id");

    let result = remove_owned_directory(trusted_base.path(), &id_root);

    assert!(
        result.is_err(),
        "a symlinked ancestor inside owned storage must refuse direct removal"
    );
    assert!(
        external_id_dir.join("keep.txt").exists(),
        "external data reached through a symlinked ancestor must survive removal"
    );
}

#[cfg(unix)]
#[test]
fn mutation_during_copy_does_not_bypass_the_frozen_byte_budget() {
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;

    let package = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    let triple = target_triple();
    write_package(package.path(), "dev.example.mutation", b"#!/bin/sh\n");
    let executable_path = package.path().join("bin").join(&triple);
    let original_length = fs::metadata(&executable_path).unwrap().len();
    let target_ino = fs::metadata(&executable_path).unwrap().ino();

    crate::plugin_host::package::test_hooks::set_after_size_observed(move |ino| {
        if ino == target_ino {
            // Grow the file in place, after its pre-copy size has already
            // been observed and budgeted against the total-byte limit, but
            // before the bounded copy reads it.
            let mut file = fs::OpenOptions::new()
                .append(true)
                .open(&executable_path)
                .unwrap();
            file.write_all(&[0xAAu8; 4096]).unwrap();
        }
    });

    let result = stage(staging.path(), package.path(), staging.path());
    crate::plugin_host::package::test_hooks::clear();

    let staged = result.expect("stage succeeds despite in-place growth after size observation");
    let staged_executable = staged.staged_path.join("bin").join(&triple);
    let staged_length = fs::metadata(&staged_executable).unwrap().len();
    assert_eq!(
        staged_length, original_length,
        "bytes appended after the size budget was observed must not appear in the staged copy"
    );
}

#[tokio::test]
async fn concurrent_disable_and_uninstall_leave_a_consistent_registry() {
    let package = tempfile::tempdir().unwrap();
    write_package(package.path(), "dev.example.concurrent", b"#!/bin/sh\n");
    let host_root = tempfile::tempdir().unwrap();
    let host = Arc::new(PluginHost::new(test_config(host_root.path())));
    host.install(package.path()).expect("install fixture");

    let disable_host = Arc::clone(&host);
    let disable = tokio::task::spawn_blocking(move || {
        disable_host.set_enabled("dev.example.concurrent", false)
    });
    let uninstall_host = Arc::clone(&host);
    let uninstall =
        tokio::task::spawn_blocking(move || uninstall_host.uninstall("dev.example.concurrent"));

    let _ = disable.await.expect("join disable");
    let _ = uninstall.await.expect("join uninstall");

    // Whichever ordering wins, the registry never ends up with more than one
    // record for this id, and a final uninstall always converges to none.
    let _ = host.uninstall("dev.example.concurrent");
    let listed = host.list().expect("list after races");
    assert!(
        listed.is_empty(),
        "no orphaned record survives the race: {listed:?}"
    );
}

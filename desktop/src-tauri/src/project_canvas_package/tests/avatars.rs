//! Published avatars. Avatars reach a frame over `__buzz/avatar/<pubkey>`
//! rather than as base64 inside an RPC message, so these bind the containment
//! that route depends on: the capability gate, the project scope, and what
//! counts as an image.

use std::path::Path;

use tempfile::TempDir;

use super::super::{
    protocol, storage::ProjectBinding, template::bundled_template, ProjectCanvasAvatarInput,
    ProjectCanvasPackageRequest, ProjectCanvasRuntime, MAX_PUBLISHED_AVATARS,
};
use super::{package_files, request, source_root, write_package, write_package_files, OWNER};

fn avatar_pubkey(index: usize) -> String {
    format!("{index:064x}")
}

fn png_bytes(len: usize) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.resize(len.max(bytes.len()), b'x');
    bytes
}

fn avatar_input(pubkey: &str, content_type: &str, bytes: &[u8]) -> ProjectCanvasAvatarInput {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    ProjectCanvasAvatarInput {
        content_type: content_type.to_string(),
        data: STANDARD.encode(bytes),
        pubkey: pubkey.to_string(),
    }
}

/// Writes the standard test package with `project.people.read` granted.
fn write_people_package(root: &Path, marker: &str) {
    let mut files = package_files(marker);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(files.get("manifest.json").unwrap()).unwrap();
    manifest["capabilities"] = serde_json::json!([
        "project.metadata.read",
        "project.channels.read",
        "project.reviews.read",
        "project.people.read"
    ]);
    files.insert(
        "manifest.json".to_string(),
        serde_json::to_vec(&manifest).unwrap(),
    );
    write_package_files(root, &files);
}

fn people_runtime(temp: &TempDir, request: ProjectCanvasPackageRequest) -> ProjectCanvasRuntime {
    let binding = ProjectBinding::parse(request).unwrap();
    write_people_package(&source_root(temp, &binding), "avatars");
    ProjectCanvasRuntime::with_root(temp.path().join("CANVASES"))
}

#[test]
fn published_avatars_are_served_by_pubkey_and_missing_ones_are_not_found() {
    let temp = TempDir::new().unwrap();
    let runtime = people_runtime(&temp, request());
    let descriptor = runtime
        .get_or_activate(request(), Some(&bundled_template().unwrap()))
        .unwrap();
    let present = avatar_pubkey(1);
    let absent = avatar_pubkey(2);
    let bytes = png_bytes(64);
    runtime
        .publish_avatars(request(), vec![avatar_input(&present, "image/png", &bytes)])
        .unwrap();

    let (content_type, body) = protocol::route(
        &runtime,
        &format!("/{}/__buzz/avatar/{present}", descriptor.load_id),
    )
    .unwrap();
    assert_eq!(content_type, "image/png");
    assert_eq!(body, bytes);

    // An unpublished person is an ordinary outcome, not an error: the SDK
    // leaves their initials in place.
    let (status, _) = protocol::route(
        &runtime,
        &format!("/{}/__buzz/avatar/{absent}", descriptor.load_id),
    )
    .unwrap_err();
    assert_eq!(status, tauri::http::StatusCode::NOT_FOUND);
}

#[test]
fn the_avatar_route_survives_a_reload_of_the_same_project() {
    let temp = TempDir::new().unwrap();
    let runtime = people_runtime(&temp, request());
    let first = runtime
        .get_or_activate(request(), Some(&bundled_template().unwrap()))
        .unwrap();
    let pubkey = avatar_pubkey(7);
    runtime
        .publish_avatars(
            request(),
            vec![avatar_input(&pubkey, "image/webp", b"RIFF\0\0\0\0WEBPxx")],
        )
        .unwrap();
    runtime.release(&first.load_id).unwrap();

    // Binds the reason the store is keyed by project rather than by load: a
    // frame that reloads must not lose every avatar until something happens to
    // republish, because a 404 here is never retried.
    let second = runtime
        .get_or_activate(request(), Some(&bundled_template().unwrap()))
        .unwrap();
    let (content_type, _) = protocol::route(
        &runtime,
        &format!("/{}/__buzz/avatar/{pubkey}", second.load_id),
    )
    .unwrap();
    assert_eq!(content_type, "image/webp");
}

#[test]
fn the_avatar_route_requires_the_people_read_capability() {
    let temp = TempDir::new().unwrap();
    let binding = ProjectBinding::parse(request()).unwrap();
    // The default package grants metadata/channels/reviews but not people.
    write_package(&source_root(&temp, &binding), "no-people");
    let runtime = ProjectCanvasRuntime::with_root(temp.path().join("CANVASES"));
    let descriptor = runtime
        .get_or_activate(request(), Some(&bundled_template().unwrap()))
        .unwrap();
    let pubkey = avatar_pubkey(3);
    runtime
        .publish_avatars(
            request(),
            vec![avatar_input(&pubkey, "image/png", &png_bytes(32))],
        )
        .unwrap();

    let (status, _) = protocol::route(
        &runtime,
        &format!("/{}/__buzz/avatar/{pubkey}", descriptor.load_id),
    )
    .unwrap_err();
    assert_eq!(status, tauri::http::StatusCode::FORBIDDEN);
}

#[test]
fn a_frame_cannot_read_another_projects_published_avatars() {
    let temp = TempDir::new().unwrap();
    let other = ProjectCanvasPackageRequest {
        community_id: "community-b".to_string(),
        project_id: format!("30621:{OWNER}:other-project"),
    };
    let runtime = people_runtime(&temp, request());
    let other_binding = ProjectBinding::parse(other.clone()).unwrap();
    write_people_package(&source_root(&temp, &other_binding), "other");
    let pubkey = avatar_pubkey(4);
    runtime
        .publish_avatars(
            request(),
            vec![avatar_input(&pubkey, "image/png", &png_bytes(32))],
        )
        .unwrap();

    let foreign = runtime
        .get_or_activate(other, Some(&bundled_template().unwrap()))
        .unwrap();
    let (status, _) = protocol::route(
        &runtime,
        &format!("/{}/__buzz/avatar/{pubkey}", foreign.load_id),
    )
    .unwrap_err();
    assert_eq!(status, tauri::http::StatusCode::NOT_FOUND);
}

#[test]
fn an_uppercase_pubkey_in_the_url_resolves_and_a_malformed_one_is_rejected() {
    let temp = TempDir::new().unwrap();
    let runtime = people_runtime(&temp, request());
    let descriptor = runtime
        .get_or_activate(request(), Some(&bundled_template().unwrap()))
        .unwrap();
    let pubkey = avatar_pubkey(0xabc);
    runtime
        .publish_avatars(
            request(),
            vec![avatar_input(
                &pubkey.to_uppercase(),
                "image/png",
                &png_bytes(32),
            )],
        )
        .unwrap();

    assert!(protocol::route(
        &runtime,
        &format!(
            "/{}/__buzz/avatar/{}",
            descriptor.load_id,
            pubkey.to_uppercase()
        ),
    )
    .is_ok());
    let (status, _) = protocol::route(
        &runtime,
        &format!("/{}/__buzz/avatar/not-a-pubkey", descriptor.load_id),
    )
    .unwrap_err();
    assert_eq!(status, tauri::http::StatusCode::BAD_REQUEST);
}

#[test]
fn publishing_rejects_types_and_bytes_that_are_not_the_image_they_claim() {
    let temp = TempDir::new().unwrap();
    let runtime = people_runtime(&temp, request());
    runtime
        .get_or_activate(request(), Some(&bundled_template().unwrap()))
        .unwrap();
    let pubkey = avatar_pubkey(5);

    let error = runtime
        .publish_avatars(
            request(),
            vec![avatar_input(&pubkey, "image/svg+xml", b"<svg/>")],
        )
        .unwrap_err();
    assert!(error.contains("unsupported project canvas avatar type"));

    // A declared type the bytes do not match is the case `nosniff` alone would
    // let through if it ever regressed.
    let error = runtime
        .publish_avatars(
            request(),
            vec![avatar_input(&pubkey, "image/png", b"<html>hi</html>")],
        )
        .unwrap_err();
    assert!(error.contains("are not image/png data"));

    let error = runtime
        .publish_avatars(
            request(),
            vec![avatar_input(&pubkey, "image/png", &png_bytes(64 * 1024))],
        )
        .unwrap_err();
    assert!(error.contains("too large"));

    let error = runtime
        .publish_avatars(
            request(),
            vec![avatar_input("beef", "image/png", &png_bytes(32))],
        )
        .unwrap_err();
    assert!(error.contains("64 hex characters"));
}

#[test]
fn a_malformed_entry_leaves_the_previously_published_avatars_intact() {
    let temp = TempDir::new().unwrap();
    let runtime = people_runtime(&temp, request());
    let descriptor = runtime
        .get_or_activate(request(), Some(&bundled_template().unwrap()))
        .unwrap();
    let good = avatar_pubkey(6);
    runtime
        .publish_avatars(
            request(),
            vec![avatar_input(&good, "image/png", &png_bytes(32))],
        )
        .unwrap();

    // The batch is validated before the store is touched, so one bad entry
    // must not cost the people already published their pictures.
    assert!(runtime
        .publish_avatars(
            request(),
            vec![
                avatar_input(&avatar_pubkey(8), "image/png", &png_bytes(32)),
                avatar_input("nope", "image/png", &png_bytes(32)),
            ],
        )
        .is_err());
    assert!(protocol::route(
        &runtime,
        &format!("/{}/__buzz/avatar/{good}", descriptor.load_id)
    )
    .is_ok());
    assert!(protocol::route(
        &runtime,
        &format!("/{}/__buzz/avatar/{}", descriptor.load_id, avatar_pubkey(8))
    )
    .is_err());
}

#[test]
fn the_avatar_store_evicts_the_oldest_entries_past_its_ceiling() {
    let temp = TempDir::new().unwrap();
    let runtime = people_runtime(&temp, request());
    let descriptor = runtime
        .get_or_activate(request(), Some(&bundled_template().unwrap()))
        .unwrap();
    let oldest = avatar_pubkey(100);
    runtime
        .publish_avatars(
            request(),
            vec![avatar_input(&oldest, "image/png", &png_bytes(32))],
        )
        .unwrap();
    for index in 0..MAX_PUBLISHED_AVATARS {
        runtime
            .publish_avatars(
                request(),
                vec![avatar_input(
                    &avatar_pubkey(200 + index),
                    "image/png",
                    &png_bytes(32),
                )],
            )
            .unwrap();
    }

    let (status, _) = protocol::route(
        &runtime,
        &format!("/{}/__buzz/avatar/{oldest}", descriptor.load_id),
    )
    .unwrap_err();
    assert_eq!(status, tauri::http::StatusCode::NOT_FOUND);
    assert!(protocol::route(
        &runtime,
        &format!(
            "/{}/__buzz/avatar/{}",
            descriptor.load_id,
            avatar_pubkey(200 + MAX_PUBLISHED_AVATARS - 1)
        )
    )
    .is_ok());
}

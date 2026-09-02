use std::{collections::BTreeMap, fs, path::Path};

use tempfile::TempDir;

use super::{
    storage::{validate_package_files, ProjectBinding, ValidatedPackage},
    ProjectCanvasPackageRequest,
};

mod avatars;
mod containment;
mod packaging;

const OWNER: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn request() -> ProjectCanvasPackageRequest {
    ProjectCanvasPackageRequest {
        community_id: "community-a".to_string(),
        project_id: format!("30621:{OWNER}:my-project"),
    }
}

fn package_files(marker: &str) -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        (
            "manifest.json".to_string(),
            serde_json::to_vec_pretty(&serde_json::json!({
                "format": "buzz-project-canvas",
                "protocolVersion": 1,
                "scripts": ["widgets/chore-board.js", "canvas.js"],
                "styles": ["styles/canvas.css"],
                "data": "data/dashboards.json",
                "capabilities": [
                    "project.metadata.read",
                    "project.channels.read",
                    "project.reviews.read"
                ]
            }))
            .unwrap(),
        ),
        (
            "widgets/chore-board.js".to_string(),
            b"globalThis.renderChores = () => {};".to_vec(),
        ),
        (
            "canvas.js".to_string(),
            format!("globalThis.canvasMarker = {marker:?};").into_bytes(),
        ),
        (
            "styles/canvas.css".to_string(),
            b"body { margin: 0; }".to_vec(),
        ),
        (
            "data/dashboards.json".to_string(),
            serde_json::to_vec(&serde_json::json!({
                "marker": marker,
                "dashboards": {
                    "test": {
                        "widgets": [{
                            "id": "chore-board",
                            "data": { "marker": marker }
                        }]
                    }
                }
            }))
            .unwrap(),
        ),
        ("assets/pixel.png".to_string(), vec![137, 80, 78, 71]),
    ])
}

fn write_package(root: &Path, marker: &str) {
    write_package_files(root, &package_files(marker));
}

fn write_package_files(root: &Path, files: &BTreeMap<String, Vec<u8>>) {
    for (relative, bytes) in files {
        let destination = root.join(relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, bytes).unwrap();
    }
}

fn template_package(marker: &str) -> ValidatedPackage {
    validate_package_files(package_files(marker)).unwrap()
}

fn source_root(temp: &TempDir, binding: &ProjectBinding) -> std::path::PathBuf {
    let root = temp.path().join("CANVASES");
    fs::create_dir_all(&root).unwrap();
    let canonical = root.canonicalize().unwrap();
    binding.project_root_for_test(&canonical)
}

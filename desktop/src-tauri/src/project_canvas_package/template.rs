//! The seed package a project's first canvas is created from, embedded in the
//! binary at compile time.
//!
//! Seeding used to read `resources/project-canvas-template` from disk, resolved
//! against the running executable. In dev builds that directory lives inside
//! the cargo target directory — shared across worktrees, rewritten by sibling
//! builds, and removed wholesale by toolchain cleanups — so when it disappeared
//! under a running app the first-ever activation of a project failed with an
//! unnamed `os error 2` and could not self-heal. There is no runtime lookup
//! left to go stale: the bytes are part of the binary, and the on-disk copy is
//! no longer bundled.

use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

use include_dir::{include_dir, Dir, DirEntry};

use super::{
    manifest::validate_relative_path,
    storage::{validate_package_files, ValidatedPackage},
};

static TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/resources/project-canvas-template");

/// The embedded template, validated through the same gate every on-disk canvas
/// package passes.
///
/// Validation hashes 2.7 MB, so the result is computed once per process and
/// shared. A template that fails validation fails every seed identically —
/// `bundled_template_seeds_a_valid_snapshot` keeps that from reaching a build.
pub(super) fn bundled_template() -> Result<Arc<ValidatedPackage>, String> {
    static VALIDATED: OnceLock<Result<Arc<ValidatedPackage>, String>> = OnceLock::new();
    VALIDATED
        .get_or_init(|| validate_package_files(template_files()?).map(Arc::new))
        .clone()
}

/// The embedded file tree, keyed by package-relative path.
///
/// `include_dir` normalizes to `/` separators on every host, and each key is
/// still run through `validate_relative_path` so the embedded template gets no
/// weaker a path gate than a package read off disk.
pub(super) fn template_files() -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut files = BTreeMap::new();
    collect_entries(TEMPLATE.entries(), &mut files)?;
    Ok(files)
}

fn collect_entries(
    entries: &[DirEntry<'_>],
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    for entry in entries {
        match entry {
            DirEntry::Dir(directory) => collect_entries(directory.entries(), files)?,
            DirEntry::File(file) => {
                let path = file
                    .path()
                    .to_str()
                    .ok_or_else(|| "project canvas package paths must be UTF-8".to_string())?;
                // Matches the on-disk scan, which ignores Finder metadata rather
                // than failing the whole package on it.
                if path.rsplit('/').next() == Some(".DS_Store") {
                    continue;
                }
                files.insert(validate_relative_path(path)?, file.contents().to_vec());
            }
        }
    }
    Ok(())
}

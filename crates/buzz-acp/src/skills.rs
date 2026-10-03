//! Materialize persona-pack skills into the agent working directory.
//!
//! Skills are files on disk: an ACP runtime finds them by scanning fixed
//! directories relative to its working directory. Buzz's canonical location is
//! `.agents/skills/<name>/`; each runtime also has its own convention, so the
//! canonical copy is linked into all of them.
//!
//! Give each agent its own working directory (`--workdir`) to keep one agent's
//! skills out of another's.

use std::io;
use std::path::{Path, PathBuf};

/// buzz-acp's canonical skill location, relative to the working directory.
const CANONICAL_DIR: &str = ".agents/skills";

/// Per-runtime skill-discovery directories, relative to the working directory.
/// Mirrors the desktop runtime catalog (`discovery/catalog.rs`).
const RUNTIME_DIRS: &[&str] = &[".claude/skills", ".goose/skills", ".codex/skills"];

/// Copy each skill directory into `<workdir>/.agents/skills/` and link it into
/// every runtime skill directory.
///
/// An existing skill of the same name is never overwritten — an operator may
/// have pinned a custom version — but its runtime links are still ensured, so a
/// pinned skill stays discoverable.
pub fn materialize(skill_dirs: &[PathBuf], workdir: &Path) -> io::Result<()> {
    let canonical_root = workdir.join(CANONICAL_DIR);

    for source in skill_dirs {
        let name = source.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("skill path has no directory name: {}", source.display()),
            )
        })?;
        let canonical = canonical_root.join(name);

        if canonical.exists() {
            tracing::warn!(
                skill = %name.to_string_lossy(),
                path = %canonical.display(),
                "skill already present; keeping the existing version"
            );
        } else {
            std::fs::create_dir_all(&canonical_root)?;
            copy_dir(source, &canonical)?;
            tracing::info!(
                skill = %name.to_string_lossy(),
                path = %canonical.display(),
                "materialized persona skill"
            );
        }

        for runtime_dir in RUNTIME_DIRS {
            let link_parent = workdir.join(runtime_dir);
            let link = link_parent.join(name);
            if link.symlink_metadata().is_ok() {
                continue;
            }
            std::fs::create_dir_all(&link_parent)?;
            link_dir(&canonical, &link)?;
        }
    }

    Ok(())
}

/// Recursively copy `src` into `dst`, creating `dst`.
fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Point `link` at `target`, falling back to a copy where a symlink cannot be
/// created (Windows without developer mode, filesystems without link support).
fn link_dir(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let result = std::os::windows::fs::symlink_dir(target, link);
    #[cfg(not(any(unix, windows)))]
    let result: io::Result<()> = Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "symlinks unsupported on this platform",
    ));

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::debug!(
                link = %link.display(),
                "symlink failed ({e}); copying the skill instead"
            );
            copy_dir(target, link)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a pack-side skill directory containing one `SKILL.md`.
    fn skill_source(root: &Path, name: &str, body: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("SKILL.md"), body).unwrap();
        std::fs::write(dir.join("nested").join("ref.md"), "nested").unwrap();
        dir
    }

    #[test]
    fn copies_skill_into_canonical_and_every_runtime_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let source = skill_source(&tmp.path().join("pack"), "security-review", "body");
        let workdir = tmp.path().join("agent");
        std::fs::create_dir_all(&workdir).unwrap();

        materialize(&[source], &workdir).unwrap();

        let canonical = workdir.join(".agents/skills/security-review");
        assert_eq!(
            std::fs::read_to_string(canonical.join("SKILL.md")).unwrap(),
            "body"
        );
        assert_eq!(
            std::fs::read_to_string(canonical.join("nested/ref.md")).unwrap(),
            "nested",
            "nested skill content must be copied too"
        );
        for runtime_dir in RUNTIME_DIRS {
            let linked = workdir.join(runtime_dir).join("security-review");
            assert_eq!(
                std::fs::read_to_string(linked.join("SKILL.md")).unwrap(),
                "body",
                "{runtime_dir} must resolve to the skill"
            );
        }
    }

    #[test]
    fn existing_skill_is_never_overwritten_but_is_still_linked() {
        let tmp = tempfile::tempdir().unwrap();
        let source = skill_source(&tmp.path().join("pack"), "search", "pack version");
        let workdir = tmp.path().join("agent");
        let canonical = workdir.join(".agents/skills/search");
        std::fs::create_dir_all(&canonical).unwrap();
        std::fs::write(canonical.join("SKILL.md"), "operator version").unwrap();

        materialize(&[source], &workdir).unwrap();

        assert_eq!(
            std::fs::read_to_string(canonical.join("SKILL.md")).unwrap(),
            "operator version",
            "a pinned skill must survive materialization"
        );
        assert_eq!(
            std::fs::read_to_string(workdir.join(".claude/skills/search/SKILL.md")).unwrap(),
            "operator version",
            "a pinned skill must still be linked into the runtime dirs"
        );
    }

    #[test]
    fn existing_runtime_link_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let source = skill_source(&tmp.path().join("pack"), "search", "pack version");
        let workdir = tmp.path().join("agent");
        let claude_skill = workdir.join(".claude/skills/search");
        std::fs::create_dir_all(&claude_skill).unwrap();
        std::fs::write(claude_skill.join("SKILL.md"), "hand-placed").unwrap();

        materialize(&[source], &workdir).unwrap();

        assert_eq!(
            std::fs::read_to_string(claude_skill.join("SKILL.md")).unwrap(),
            "hand-placed"
        );
    }

    #[test]
    fn materializing_twice_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let source = skill_source(&tmp.path().join("pack"), "search", "body");
        let workdir = tmp.path().join("agent");

        materialize(std::slice::from_ref(&source), &workdir).unwrap();
        materialize(&[source], &workdir).unwrap();

        assert_eq!(
            std::fs::read_to_string(workdir.join(".goose/skills/search/SKILL.md")).unwrap(),
            "body"
        );
    }
}

use chrono::Utc;

pub fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

/// Deserialize an `Option<Option<T>>` field that distinguishes an absent key
/// from an explicit JSON `null`.
///
/// Plain serde collapses a present `null` into the outer `None`, making
/// "clear this field" indistinguishable from "leave it unchanged". Paired with
/// `#[serde(default)]`, this yields the tri-state needed for nullable patches:
/// absent → `None`, `null` → `Some(None)`, value → `Some(Some(value))`.
pub fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(deserializer).map(Some)
}

/// Turn a human-readable name into a filesystem-safe slug.
///
/// Non-alphanumeric characters become hyphens, leading/trailing hyphens are
/// stripped, and the result is capped at `max_len` characters (on a hyphen
/// boundary when possible). Returns `fallback` when the input produces an
/// empty slug.
pub fn slugify(name: &str, fallback: &str, max_len: usize) -> String {
    let raw: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let raw = if raw.is_empty() { fallback } else { &raw };
    let raw = if raw.len() > max_len {
        &raw[..max_len]
    } else {
        raw
    };
    raw.trim_end_matches('-').to_string()
}

// ── Safe symlink utilities ────────────────────────────────────────────────────

/// Create a symlink at `link` pointing to `target` on Unix; no-op on Windows.
///
/// Worktree sync and nest setup are Unix-only features. This wrapper lets
/// call sites compile and run harmlessly on non-Unix platforms.
#[cfg(unix)]
pub(crate) fn create_symlink(
    target: &std::path::Path,
    link: &std::path::Path,
) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// No-op on non-Unix platforms.
#[cfg(not(unix))]
pub(crate) fn create_symlink(
    _target: &std::path::Path,
    _link: &std::path::Path,
) -> std::io::Result<()> {
    Ok(())
}

/// Returns `true` when `link` is a symlink whose stored target equals `target`.
///
/// Compares the raw stored link value — no canonicalization — so relative
/// targets (e.g. `../../.agents/skills/buzz-cli`) compare correctly against
/// the literal string used to create them.
pub(crate) fn symlink_points_to(link: &std::path::Path, target: &std::path::Path) -> bool {
    link.is_symlink()
        && std::fs::read_link(link)
            .map(|t| t == target)
            .unwrap_or(false)
}

/// Compute a collision-safe backup path for `dst`.
///
/// The candidate is `<parent>/<full-filename>.bak.<ms-timestamp>`. If that
/// path already exists (rare — same-millisecond collision or leftover backup),
/// appends `-2`, `-3`, … up to 100. Returns `None` when all 100 candidates
/// are occupied, indicating a backup failure.
pub(crate) fn backup_path(dst: &std::path::Path) -> Option<std::path::PathBuf> {
    let name = dst.file_name()?.to_str()?;
    let stamp = Utc::now().format("%Y%m%d-%H%M%S%.3f");
    let base = format!("{name}.bak.{stamp}");
    let parent = dst.parent()?;

    let candidate = parent.join(&base);
    if !candidate.exists() {
        return Some(candidate);
    }
    // Collision — try suffixes -2 … -100.
    for n in 2u32..=100 {
        let candidate = parent.join(format!("{base}-{n}"));
        if !candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Replace `dst` with a symlink pointing to `src`, backing up any real
/// file or directory at `dst` first.
///
/// Behaviour by what `dst` currently is:
///
/// - **Already a correct symlink** (stored target == `src`): no-op, returns 0.
/// - **Wrong or broken symlink**: remove and replace; no backup — a symlink
///   holds no user data. Returns 1 on success, 0 if removal fails (the
///   subsequent `create_symlink` will surface EEXIST).
/// - **Real file or real directory**: rename to
///   `<full-filename>.bak.<ms-timestamp>` (collision-safe, up to suffix -100).
///   If backup fails, `dst` is left untouched and 0 is returned. If the
///   backup succeeds but `create_symlink` fails, the backup is renamed back to
///   restore `dst`; if that rollback also fails, an actionable error is logged.
///   Returns 1 on success, 0 on any failure.
/// - **Absent**: creates the symlink and returns 1.
#[cfg(unix)]
pub(crate) fn replace_with_symlink(src: &std::path::Path, dst: &std::path::Path) -> u32 {
    if dst.is_symlink() {
        if symlink_points_to(dst, src) {
            return 0;
        }
        // Wrong or broken symlink — remove and replace, no backup.
        if let Err(e) = std::fs::remove_file(dst) {
            eprintln!(
                "buzz-desktop: symlink-util: failed to remove stale symlink {}: {e}",
                dst.display()
            );
            // Fall through — create_symlink will surface EEXIST.
        }
    } else if dst.exists() {
        // Real file or real directory — back up before replacing.
        let label = if dst.is_dir() { "dir" } else { "file" };
        let Some(bak) = backup_path(dst) else {
            eprintln!(
                "buzz-desktop: symlink-util: all backup paths occupied for {}; skipping",
                dst.display()
            );
            return 0;
        };
        match std::fs::rename(dst, &bak) {
            Ok(()) => eprintln!(
                "buzz-desktop: symlink-util: backed up real {label} {} → {}",
                dst.display(),
                bak.display()
            ),
            Err(e) => {
                eprintln!(
                    "buzz-desktop: symlink-util: failed to back up {label} {}: {e}",
                    dst.display()
                );
                return 0;
            }
        }
        // Backup succeeded — attempt symlink creation.
        if let Err(e) = create_symlink(src, dst) {
            eprintln!(
                "buzz-desktop: symlink-util: failed to symlink {} → {}: {e}; attempting rollback",
                dst.display(),
                src.display()
            );
            if let Err(rb_err) = std::fs::rename(&bak, dst) {
                eprintln!(
                    "buzz-desktop: symlink-util: ROLLBACK FAILED ({rb_err}) — \
                     {dst_disp} is still at {bak_disp}; \
                     restore it manually: `mv {bak_disp} {dst_disp}`",
                    dst_disp = dst.display(),
                    bak_disp = bak.display(),
                );
            }
            return 0;
        }
        return 1;
    }

    // dst was absent or was a symlink (already removed above).
    match create_symlink(src, dst) {
        Ok(()) => 1,
        Err(e) => {
            eprintln!(
                "buzz-desktop: symlink-util: failed to symlink {} → {}: {e}",
                dst.display(),
                src.display()
            );
            0
        }
    }
}

/// No-op on non-Unix platforms — always returns 0.
#[cfg(not(unix))]
pub(crate) fn replace_with_symlink(_src: &std::path::Path, _dst: &std::path::Path) -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn double_option_tristate() {
        #[derive(serde::Deserialize)]
        struct P {
            #[serde(default, deserialize_with = "super::double_option")]
            ttl: Option<Option<i32>>,
        }
        let absent: P = serde_json::from_str("{}").unwrap();
        let null: P = serde_json::from_str(r#"{"ttl": null}"#).unwrap();
        let set: P = serde_json::from_str(r#"{"ttl": 3600}"#).unwrap();
        assert_eq!(absent.ttl, None);
        assert_eq!(null.ttl, Some(None));
        assert_eq!(set.ttl, Some(Some(3600)));
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("My Cool Team", "team", 50), "my-cool-team");
    }

    #[test]
    fn slugify_special_chars() {
        assert_eq!(slugify("héllo wörld!", "fallback", 50), "h-llo-w-rld");
    }

    #[test]
    fn slugify_empty_uses_fallback() {
        assert_eq!(slugify("   ", "persona", 50), "persona");
        assert_eq!(slugify("", "team", 50), "team");
    }

    #[test]
    fn slugify_truncates_at_max_len() {
        let long_name = "a]".repeat(60);
        let result = slugify(&long_name, "fallback", 10);
        assert!(result.len() <= 10);
        assert!(!result.ends_with('-'));
    }

    #[test]
    fn slugify_trims_trailing_hyphens_after_truncation() {
        // "abcde-----fghij" truncated at 10 → "abcde-----" → trimmed → "abcde"
        assert_eq!(slugify("abcde     fghij", "x", 10), "abcde");
    }

    // ── symlink_util ──────────────────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn replace_with_symlink_backs_up_real_file_and_creates_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.json");
        let dst = dir.path().join("dest.json");

        std::fs::write(&src, r#"[{"id":"canonical"}]"#).unwrap();
        std::fs::write(&dst, r#"[{"id":"real-local-data"}]"#).unwrap();

        let created = super::replace_with_symlink(&src, &dst);
        assert_eq!(created, 1);

        assert!(dst.is_symlink());
        assert_eq!(std::fs::read_link(&dst).unwrap(), src);

        let bak_entry = std::fs::read_dir(dir.path()).unwrap().flatten().find(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.starts_with("dest.json.bak."))
                .unwrap_or(false)
        });
        assert!(bak_entry.is_some(), "a .bak.* backup file must exist");
        let bak_content = std::fs::read_to_string(bak_entry.unwrap().path()).unwrap();
        assert_eq!(bak_content, r#"[{"id":"real-local-data"}]"#);
    }

    #[cfg(unix)]
    #[test]
    fn replace_with_symlink_backs_up_real_dir_and_creates_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("canonical-teams");
        let dst = dir.path().join("local-teams");

        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(dst.join("stale.txt"), "old-content").unwrap();

        let created = super::replace_with_symlink(&src, &dst);
        assert_eq!(created, 1);

        assert!(dst.is_symlink());
        assert_eq!(std::fs::read_link(&dst).unwrap(), src);

        let bak_entry = std::fs::read_dir(dir.path()).unwrap().flatten().find(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.starts_with("local-teams.bak."))
                .unwrap_or(false)
        });
        assert!(bak_entry.is_some(), "a .bak.* backup directory must exist");
        let bak_path = bak_entry.unwrap().path();
        assert!(bak_path.is_dir(), "backup must be a directory");
        assert_eq!(
            std::fs::read_to_string(bak_path.join("stale.txt")).unwrap(),
            "old-content"
        );
    }

    #[cfg(unix)]
    #[test]
    fn replace_with_symlink_noop_when_already_correct_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.json");
        let dst = dir.path().join("dest.json");

        std::fs::write(&src, "data").unwrap();
        std::os::unix::fs::symlink(&src, &dst).unwrap();

        let created = super::replace_with_symlink(&src, &dst);
        assert_eq!(created, 0);

        assert!(dst.is_symlink());
        assert_eq!(std::fs::read_link(&dst).unwrap(), src);
        let bak_count = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .map(|n| n.contains(".bak."))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(
            bak_count, 0,
            "no backup should be created for a correct symlink"
        );
    }

    #[cfg(unix)]
    #[test]
    fn replace_with_symlink_replaces_wrong_symlink_without_backup() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.json");
        let dst = dir.path().join("dest.json");
        let wrong_target = dir.path().join("wrong.json");

        std::fs::write(&src, "data").unwrap();
        std::fs::write(&wrong_target, "wrong").unwrap();
        std::os::unix::fs::symlink(&wrong_target, &dst).unwrap();

        let created = super::replace_with_symlink(&src, &dst);
        assert_eq!(created, 1);

        assert!(dst.is_symlink());
        assert_eq!(std::fs::read_link(&dst).unwrap(), src);

        let bak_count = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .map(|n| n.contains(".bak."))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(bak_count, 0, "symlinks must not produce backups");
    }

    #[cfg(unix)]
    #[test]
    fn replace_with_symlink_replaces_broken_symlink_without_backup() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.json");
        let dst = dir.path().join("dest.json");

        std::fs::write(&src, "data").unwrap();
        std::os::unix::fs::symlink(dir.path().join("nonexistent.json"), &dst).unwrap();

        let created = super::replace_with_symlink(&src, &dst);
        assert_eq!(created, 1);

        assert!(dst.is_symlink());
        assert_eq!(std::fs::read_link(&dst).unwrap(), src);

        let bak_count = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .map(|n| n.contains(".bak."))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(bak_count, 0, "broken symlinks must not produce backups");
    }
}

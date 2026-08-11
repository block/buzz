//! Workdir validation — the load-bearing security boundary for this crate.
//!
//! A channel's declared working directory is attacker-influenced (anyone who
//! can create a channel controls this string), so it must be canonicalized
//! and checked against an explicit allowlist of roots *before* anything is
//! spawned there. There is no default allowed root — the operator must pass
//! at least one `--allowed-root`.

use std::path::{Path, PathBuf};

pub struct AllowedRoots(Vec<PathBuf>);

impl AllowedRoots {
    pub fn new(roots: Vec<PathBuf>) -> anyhow::Result<Self> {
        if roots.is_empty() {
            anyhow::bail!("at least one --allowed-root is required");
        }
        let canonical = roots
            .iter()
            .map(|r| {
                r.canonicalize()
                    .map_err(|e| anyhow::anyhow!("--allowed-root {}: {e}", r.display()))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self(canonical))
    }

    /// Expand a `~`/`~/...` prefix using `$HOME`, leaving other paths as-is.
    fn expand_tilde(raw: &str) -> anyhow::Result<PathBuf> {
        if raw == "~" || raw.starts_with("~/") {
            let home = std::env::var("HOME")
                .map_err(|_| anyhow::anyhow!("workdir uses ~ but $HOME is not set"))?;
            let rest = raw.strip_prefix('~').unwrap_or("").trim_start_matches('/');
            return Ok(Path::new(&home).join(rest));
        }
        Ok(PathBuf::from(raw))
    }

    /// Validate a raw workdir string from a channel description.
    ///
    /// Returns the canonicalized path on success, or a human-readable
    /// rejection reason (suitable for posting back into the channel) on
    /// failure.
    pub fn validate(&self, raw: &str) -> Result<PathBuf, String> {
        let expanded = Self::expand_tilde(raw).map_err(|e| format!("{e} (workdir: {raw})"))?;
        if !expanded.is_absolute() {
            return Err(format!("not an absolute path (or ~/...): {raw}"));
        }
        let resolved = expanded
            .canonicalize()
            .map_err(|_| format!("path does not exist: {}", expanded.display()))?;
        if !resolved.is_dir() {
            return Err(format!("not a directory: {}", resolved.display()));
        }
        if self.0.iter().any(|root| resolved.starts_with(root)) {
            Ok(resolved)
        } else {
            Err(format!(
                "outside allowed roots ({}): {}",
                self.0
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                resolved.display()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_outside_allowed_roots() {
        let tmp = std::env::temp_dir();
        let roots = AllowedRoots::new(vec![tmp.join("buzz-supervisor-test-root")]).ok();
        // Root doesn't exist yet in a fresh test env — construct directly
        // instead to keep this test hermetic.
        let _ = roots;
        let allowed = AllowedRoots(vec![tmp.canonicalize().unwrap()]);
        assert!(allowed.validate("/etc").is_err());
    }

    #[test]
    fn accepts_paths_inside_allowed_roots() {
        let tmp = std::env::temp_dir().canonicalize().unwrap();
        let allowed = AllowedRoots(vec![tmp.clone()]);
        assert_eq!(allowed.validate(tmp.to_str().unwrap()).unwrap(), tmp);
    }
}

use serde::Serialize;
use std::path::{Path, PathBuf};

/// A single installed skill discovered on disk.
///
/// Feeds the desktop "Skill selection" panel. Each skill is a directory that
/// contains a `SKILL.md` whose YAML frontmatter provides the human `name` and
/// `description` shown in the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledSkill {
    /// Skill `name` from SKILL.md frontmatter.
    pub name: String,
    /// One-line `description` from frontmatter (may be empty).
    pub description: String,
    /// Absolute path to the skill directory on disk.
    pub path: String,
    /// Which skill root it came from (e.g. `.agents/skills`).
    pub source: String,
}

/// Relative skill roots (under `$HOME`) that Buzz scans for installed skills.
///
/// These are the union of every runtime skill location: the canonical
/// `.agents/skills` nest home, the per-runtime codex / claude / goose folders,
/// and the bundled `buzz-cli` skill. Order matters for first-wins dedup only.
const SKILL_ROOTS: &[&str] = &[
    ".agents/skills",
    ".codex/skills",
    ".claude/skills",
    ".goose/skills",
    ".buzz/.agents/skills",
];

/// List every installed skill across the known runtime skill roots.
///
/// Returns a de-duplicated, name-sorted list. A missing `HOME` yields an empty
/// list; missing roots are skipped silently.
#[tauri::command]
pub fn list_installed_skills() -> Vec<InstalledSkill> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    let mut skills: Vec<InstalledSkill> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for root in SKILL_ROOTS {
        scan_skill_root(&home.join(root), root, &mut seen, &mut skills);
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn scan_skill_root(
    root: &Path,
    source: &str,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<InstalledSkill>,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let skill_md = entry.path().join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&skill_md) else {
            continue;
        };
        let Some((name, description)) = parse_frontmatter(&content) else {
            continue;
        };
        // Deduplicate by name across roots; the first root in SKILL_ROOTS wins.
        if !seen.insert(name.clone()) {
            continue;
        }
        out.push(InstalledSkill {
            name,
            description,
            path: entry.path().to_string_lossy().into_owned(),
            source: source.to_string(),
        });
    }
}

/// Parse `name` (required) and `description` (optional) from the leading YAML
/// frontmatter block of a SKILL.md. Returns `None` when there is no frontmatter
/// or no `name:` field — such directories are not treated as skills.
fn parse_frontmatter(content: &str) -> Option<(String, String)> {
    let body = content.strip_prefix("---\n")?;
    let block = body.split("\n---").next()?;
    let mut name: Option<String> = None;
    let mut description = String::new();
    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("name:") {
            name = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("description:") {
            description = rest.trim().to_string();
        }
    }
    name.map(|n| (n, description))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "buzz-skills-test-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_skill_with_frontmatter(path: &Path, name: &str, description: &str) {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(
            path.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\nBody.\n"),
        )
        .unwrap();
    }

    #[test]
    fn lists_skills_across_roots_dedupes_and_sorts() {
        let home = temp_home("roots");
        let agents = home.join(".agents/skills");
        let codex = home.join(".codex/skills");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::create_dir_all(&codex).unwrap();
        // Same name in two roots -> first root (.agents) wins.
        write_skill_with_frontmatter(&agents.join("shared"), "shared", "from agents");
        write_skill_with_frontmatter(&codex.join("shared"), "shared", "from codex");
        write_skill_with_frontmatter(&codex.join("zeta"), "zeta", "Z skill");
        // A directory without SKILL.md is ignored.
        std::fs::create_dir_all(&codex.join("no-skill")).unwrap();

        let env_guard = std::env::set_var("HOME", &home);
        let skills = list_installed_skills();
        drop(env_guard);

        assert_eq!(skills.len(), 2, "shared deduped, zeta present");
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["shared", "zeta"], "name-sorted");
        let shared = &skills[0];
        assert_eq!(shared.description, "from agents", "first root wins");
        assert_eq!(shared.source, ".agents/skills");
    }

    #[test]
    fn skills_without_name_are_skipped() {
        let home = temp_home("noname");
        let dir = home.join(".agents/skills/anon");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\ndescription: no name here\n---\nBody.\n",
        )
        .unwrap();
        let env_guard = std::env::set_var("HOME", &home);
        let skills = list_installed_skills();
        drop(env_guard);
        assert!(skills.is_empty());
    }

    #[test]
    fn missing_home_returns_empty() {
        std::env::remove_var("HOME");
        assert!(list_installed_skills().is_empty());
    }
}

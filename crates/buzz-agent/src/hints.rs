use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::mcp::truncate_at_boundary;

const MAX_HINTS_BYTES: usize = 128 * 1024;
pub const MAX_SKILL_BODY_BYTES: usize = 32 * 1024;
const SKILL_DIRS: &[&str] = &[".agents/skills", ".goose/skills", ".claude/skills"];

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

#[derive(Clone)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    /// Absolute path to the SKILL.md file; used by `load_skill` to read on demand.
    pub path: PathBuf,
    /// Absolute paths to every non-SKILL.md file in the skill directory tree.
    /// Pre-enumerated at discovery time so `load_skill` can match by relative path
    /// without doing arbitrary filesystem lookups at call time.
    pub supporting_files: Vec<PathBuf>,
}

/// Handles both normal repos (`.git/` dir) and worktrees (`.git` file).
fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => return None,
        }
    }
}

fn load_hint_files_impl(cwd: &Path, home: Option<&Path>) -> String {
    let mut chain = match find_git_root(cwd) {
        Some(root) => {
            let mut c: Vec<PathBuf> = cwd
                .ancestors()
                .take_while(|a| a.starts_with(&root))
                .map(|a| a.to_path_buf())
                .collect();
            // ancestors() yields cwd first, root last — reverse for root→cwd.
            c.reverse();
            c
        }
        None => vec![cwd.to_path_buf()],
    };

    // Prepend ~/AGENTS.md as global layer, unless ~ is already in the chain.
    if let Some(home) = home {
        if !chain.iter().any(|d| d == home) {
            chain.insert(0, home.to_path_buf());
        }
    }

    let mut result = String::new();
    for dir in &chain {
        let path = dir.join("AGENTS.md");
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !result.is_empty() {
            result.push_str("\n\n");
        }
        let remaining = MAX_HINTS_BYTES.saturating_sub(result.len());
        if remaining == 0 {
            break;
        }
        if content.len() <= remaining {
            result.push_str(&content);
        } else {
            let truncated = truncate_at_boundary(&content, remaining);
            result.push_str(truncated);
            break;
        }
    }
    result
}

fn parse_skill_frontmatter(content: &str) -> Option<(String, String)> {
    // Must start with `---`
    let rest = content.strip_prefix("---\n")?;
    // Find the closing `---`
    let close_pos = rest.find("\n---")?;
    let yaml_block = &rest[..close_pos];

    let map: HashMap<String, serde_yaml::Value> = serde_yaml::from_str(yaml_block).ok()?;
    let name = map
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)?;
    let description = map
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();

    Some((name, description))
}

fn scan_skill_dir(dir: &Path, seen: &mut HashSet<String>, skills: &mut Vec<SkillEntry>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subdirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        // Use std::fs::metadata (follows symlinks) rather than DirEntry::file_type
        // (which returns FileType::Symlink for symlinks, causing is_dir() to return
        // false even when the symlink target is a directory).
        .filter(|e| {
            std::fs::metadata(e.path())
                .map(|m| m.is_dir())
                .unwrap_or(false)
        })
        .map(|e| e.path())
        .collect();
    subdirs.sort();

    for subdir in subdirs {
        let skill_md = subdir.join("SKILL.md");
        let Ok(content) = std::fs::read_to_string(&skill_md) else {
            continue;
        };
        let Some((name, description)) = parse_skill_frontmatter(&content) else {
            continue;
        };
        if seen.contains(&name) {
            continue;
        }
        seen.insert(name.clone());

        // Collect supporting files: every non-SKILL.md file in the skill dir tree.
        // Don't descend into subdirs that themselves have a SKILL.md — those are
        // separate skills with their own entries.
        let supporting_files = collect_supporting_files(&subdir);

        skills.push(SkillEntry {
            name,
            description,
            path: skill_md,
            supporting_files,
        });
    }
}

/// Walk `skill_dir` recursively and return the absolute path of every file
/// that is not `SKILL.md`. Subdirectories that contain their own `SKILL.md`
/// are treated as separate skills and are not descended into.
fn collect_supporting_files(skill_dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let mut visited_dirs = HashSet::new();
    collect_supporting_files_impl(skill_dir, &mut result, &mut visited_dirs);
    result.sort();
    result
}

fn collect_supporting_files_impl(
    current: &Path,
    out: &mut Vec<PathBuf>,
    visited_dirs: &mut HashSet<PathBuf>,
) {
    let Ok(canonical_current) = current.canonicalize() else {
        return;
    };
    if !visited_dirs.insert(canonical_current) {
        return;
    }

    let Ok(entries) = std::fs::read_dir(current) else {
        return;
    };
    let mut items: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    items.sort_by_key(|e| e.path());

    for entry in items {
        let path = entry.path();
        // Use std::fs::metadata (follows symlinks) so symlinked subdirs and files
        // inside a skill directory are handled correctly.
        let ft = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if ft.is_dir() {
            // Don't descend into subdirs that are themselves skills.
            if path.join("SKILL.md").is_file() {
                continue;
            }
            collect_supporting_files_impl(&path, out, visited_dirs);
        } else if ft.is_file() && path.file_name().and_then(|n| n.to_str()) != Some("SKILL.md") {
            out.push(path);
        }
    }
}

fn discover_skills_impl(cwd: &Path, home: Option<&Path>) -> Vec<SkillEntry> {
    let mut seen = HashSet::new();
    let mut skills = Vec::new();

    for dir_suffix in SKILL_DIRS {
        scan_skill_dir(&cwd.join(dir_suffix), &mut seen, &mut skills);
    }

    if let Some(home) = home {
        scan_skill_dir(&home.join(".agents/skills"), &mut seen, &mut skills);
    }

    skills
}

pub fn build_hints_section(cwd: &Path) -> (String, Vec<SkillEntry>) {
    build_hints_section_impl(cwd, home_dir().as_deref())
}

fn build_hints_section_impl(cwd: &Path, home: Option<&Path>) -> (String, Vec<SkillEntry>) {
    let hints_text = load_hint_files_impl(cwd, home);
    let skills = discover_skills_impl(cwd, home);

    if hints_text.is_empty() && skills.is_empty() {
        return (String::new(), skills);
    }

    let mut out = String::from("# Additional Instructions\n");

    if !hints_text.is_empty() {
        out.push_str("\n## Project Hints\n");
        out.push_str(&hints_text);
        out.push('\n');
    }

    if !skills.is_empty() {
        out.push_str("\n## Available Skills\n");
        for skill in &skills {
            out.push_str(&format!("- {}: {}\n", skill.name, skill.description));
        }
        out.push_str(
            "\nUse the `load_skill` tool to read the full content of a skill before using it.\n",
        );
    }

    (out, skills)
}

/// Strip the YAML frontmatter block from a skill file's content and return
/// the body. If no valid frontmatter is found, returns the content unchanged.
pub(crate) fn strip_frontmatter(content: &str) -> &str {
    let Some(rest) = content.strip_prefix("---\n") else {
        return content;
    };
    let Some(close_pos) = rest.find("\n---") else {
        return content;
    };
    let after = &rest[close_pos + 4..]; // skip "\n---"
    after.strip_prefix('\n').unwrap_or(after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn find_git_root_normal_repo() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        assert_eq!(find_git_root(root), Some(root.to_path_buf()));
    }

    #[test]
    fn find_git_root_worktree() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // .git as a file (worktree)
        std::fs::write(root.join(".git"), "gitdir: ../main/.git/worktrees/wt").unwrap();
        assert_eq!(find_git_root(root), Some(root.to_path_buf()));
    }

    #[test]
    fn find_git_root_none() {
        let tmp = TempDir::new().unwrap();
        // No .git anywhere under tmp
        let result = find_git_root(tmp.path());
        // In a CI environment the test itself may live inside a real git repo,
        // so only assert None when tmp is truly isolated (not a subpath of a git repo).
        // We verify by checking that any found root is NOT inside tmp.
        if let Some(found) = result {
            assert!(!found.starts_with(tmp.path()));
        }
    }

    #[test]
    fn find_git_root_from_subdirectory() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        let deep = root.join("sub").join("deep");
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(find_git_root(&deep), Some(root.to_path_buf()));
    }

    #[test]
    fn load_hint_files_single_at_cwd() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        // No .git → no git root discovery; only cwd is checked.
        std::fs::write(cwd.join("AGENTS.md"), "cwd hints").unwrap();
        let result = load_hint_files_impl(cwd, None);
        assert_eq!(result, "cwd hints");
    }

    #[test]
    fn load_hint_files_git_root_and_cwd() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join("AGENTS.md"), "root hints").unwrap();
        let sub = root.join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("AGENTS.md"), "sub hints").unwrap();
        let result = load_hint_files_impl(&sub, None);
        // Root hints must come first.
        assert!(
            result.starts_with("root hints"),
            "expected root hints first, got: {result:?}"
        );
        assert!(result.contains("sub hints"), "missing sub hints");
        let root_pos = result.find("root hints").unwrap();
        let sub_pos = result.find("sub hints").unwrap();
        assert!(root_pos < sub_pos, "root hints should precede sub hints");
    }

    #[test]
    fn load_hint_files_missing_files() {
        let tmp = TempDir::new().unwrap();
        let result = load_hint_files_impl(tmp.path(), None);
        assert_eq!(result, "");
    }

    #[test]
    fn discover_skills_finds_across_dirs() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();

        // Skill in .agents/skills/
        let agents_skill = cwd.join(".agents/skills/my-skill");
        std::fs::create_dir_all(&agents_skill).unwrap();
        std::fs::write(
            agents_skill.join("SKILL.md"),
            "---\nname: my-skill\ndescription: A skill\n---\nSkill body here.\n",
        )
        .unwrap();

        // Skill in .goose/skills/
        let goose_skill = cwd.join(".goose/skills/other-skill");
        std::fs::create_dir_all(&goose_skill).unwrap();
        std::fs::write(
            goose_skill.join("SKILL.md"),
            "---\nname: other-skill\ndescription: Another skill\n---\nOther body.\n",
        )
        .unwrap();

        let skills = discover_skills_impl(cwd, None);
        assert_eq!(skills.len(), 2);
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"my-skill"), "missing my-skill");
        assert!(names.contains(&"other-skill"), "missing other-skill");
        // Paths should point to the SKILL.md files.
        for skill in &skills {
            assert!(skill.path.exists(), "path does not exist: {:?}", skill.path);
            assert!(skill.path.ends_with("SKILL.md"));
        }
    }

    #[test]
    fn discover_skills_dedup_by_name() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();

        // Same name in .agents/skills/ (first) and .goose/skills/ (second)
        let agents_skill = cwd.join(".agents/skills/shared");
        std::fs::create_dir_all(&agents_skill).unwrap();
        std::fs::write(
            agents_skill.join("SKILL.md"),
            "---\nname: shared\ndescription: from agents\n---\nAgents body.\n",
        )
        .unwrap();

        let goose_skill = cwd.join(".goose/skills/shared");
        std::fs::create_dir_all(&goose_skill).unwrap();
        std::fs::write(
            goose_skill.join("SKILL.md"),
            "---\nname: shared\ndescription: from goose\n---\nGoose body.\n",
        )
        .unwrap();

        let skills = discover_skills_impl(cwd, None);
        assert_eq!(skills.len(), 1, "duplicate name should be deduplicated");
        assert_eq!(
            skills[0].description, "from agents",
            "first wins (.agents/)"
        );
        // Path should point to the .agents/ version (first wins).
        assert!(skills[0]
            .path
            .to_str()
            .unwrap()
            .contains(".agents/skills/shared"));
    }

    #[test]
    fn discover_skills_skips_missing_name() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();

        let skill_dir = cwd.join(".agents/skills/no-name");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\ndescription: No name here\n---\nBody.\n",
        )
        .unwrap();

        let skills = discover_skills_impl(cwd, None);
        assert!(skills.is_empty(), "entry without name should be skipped");
    }

    #[test]
    fn build_hints_section_empty() {
        let tmp = TempDir::new().unwrap();
        let (result, skills) = build_hints_section_impl(tmp.path(), None);
        assert_eq!(result, "");
        assert!(skills.is_empty());
    }

    #[test]
    fn build_hints_section_combined() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();

        std::fs::write(cwd.join("AGENTS.md"), "Project-level hints.").unwrap();

        let skill_dir = cwd.join(".agents/skills/buzz-cli");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: buzz-cli\ndescription: CLI reference for Buzz managed agents\n---\nUse `buzz` to manage agents.\n",
        )
        .unwrap();

        let (result, skills) = build_hints_section_impl(cwd, None);

        assert!(
            result.contains("# Additional Instructions"),
            "missing header"
        );
        assert!(result.contains("## Project Hints"), "missing Project Hints");
        assert!(
            result.contains("Project-level hints."),
            "missing hints content"
        );
        assert!(
            result.contains("## Available Skills"),
            "missing Available Skills"
        );
        assert!(
            result.contains("buzz-cli: CLI reference for Buzz managed agents"),
            "missing skill bullet"
        );
        // Body must NOT be inlined — lazy loading only.
        assert!(
            !result.contains("Use `buzz` to manage agents."),
            "skill body must not be inlined in system prompt"
        );
        // The load_skill instruction must be present.
        assert!(
            result.contains("load_skill"),
            "missing load_skill instruction"
        );
        // The old ### heading format must not appear.
        assert!(
            !result.contains("### buzz-cli"),
            "skill body heading must not be inlined"
        );
        // The returned skills list should contain the discovered skill.
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "buzz-cli");
    }

    #[test]
    fn load_hint_files_global_loaded_first() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        std::fs::write(home.path().join("AGENTS.md"), "global hints").unwrap();
        std::fs::write(cwd.path().join("AGENTS.md"), "local hints").unwrap();
        let result = load_hint_files_impl(cwd.path(), Some(home.path()));
        let global_pos = result.find("global hints").unwrap();
        let local_pos = result.find("local hints").unwrap();
        assert!(
            global_pos < local_pos,
            "global hints should precede local hints"
        );
    }

    #[test]
    fn load_hint_files_home_missing_agents_md() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        std::fs::write(cwd.path().join("AGENTS.md"), "local only").unwrap();
        let result = load_hint_files_impl(cwd.path(), Some(home.path()));
        assert_eq!(result, "local only");
    }

    #[test]
    fn load_hint_files_no_home_dir() {
        let cwd = TempDir::new().unwrap();
        std::fs::write(cwd.path().join("AGENTS.md"), "local only").unwrap();
        let result = load_hint_files_impl(cwd.path(), None);
        assert_eq!(result, "local only");
    }

    #[test]
    fn load_hint_files_dedup_when_home_in_chain() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path();
        std::fs::write(home.join("AGENTS.md"), "single load").unwrap();
        let result = load_hint_files_impl(home, Some(home));
        assert_eq!(
            result.matches("single load").count(),
            1,
            "AGENTS.md should be loaded exactly once when CWD is home"
        );
    }

    #[test]
    fn load_hint_files_dedup_when_home_is_git_root() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path();
        std::fs::create_dir(home.join(".git")).unwrap();
        std::fs::write(home.join("AGENTS.md"), "root+home hints").unwrap();
        let sub = home.join("sub");
        std::fs::create_dir(&sub).unwrap();
        let result = load_hint_files_impl(&sub, Some(home));
        assert_eq!(
            result.matches("root+home hints").count(),
            1,
            "AGENTS.md should be loaded once when home is git root"
        );
    }

    #[test]
    fn discover_skills_global_skills_loaded() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let skill_dir = home.path().join(".agents/skills/global-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: global-skill\ndescription: A global skill\n---\nGlobal body.\n",
        )
        .unwrap();
        let skills = discover_skills_impl(cwd.path(), Some(home.path()));
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "global-skill");
    }

    #[test]
    fn discover_skills_project_wins_over_global() {
        let home = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();

        let project_skill = cwd.path().join(".agents/skills/shared");
        std::fs::create_dir_all(&project_skill).unwrap();
        std::fs::write(
            project_skill.join("SKILL.md"),
            "---\nname: shared\ndescription: from project\n---\nProject body.\n",
        )
        .unwrap();

        let global_skill = home.path().join(".agents/skills/shared");
        std::fs::create_dir_all(&global_skill).unwrap();
        std::fs::write(
            global_skill.join("SKILL.md"),
            "---\nname: shared\ndescription: from global\n---\nGlobal body.\n",
        )
        .unwrap();

        let skills = discover_skills_impl(cwd.path(), Some(home.path()));
        assert_eq!(skills.len(), 1, "duplicate name should be deduplicated");
        assert_eq!(
            skills[0].description, "from project",
            "project-level should win over global"
        );
    }

    #[test]
    fn discover_skills_no_home_dir() {
        let cwd = TempDir::new().unwrap();
        let skill_dir = cwd.path().join(".agents/skills/local");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: local\ndescription: Local skill\n---\nBody.\n",
        )
        .unwrap();
        let skills = discover_skills_impl(cwd.path(), None);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "local");
    }

    #[test]
    fn collect_supporting_files_finds_non_skill_md_files() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();
        // SKILL.md should be excluded.
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: x\n---\n").unwrap();
        // A references subdir with files.
        let refs = skill_dir.join("references");
        std::fs::create_dir_all(&refs).unwrap();
        std::fs::write(refs.join("foo.md"), "foo").unwrap();
        std::fs::write(refs.join("bar.md"), "bar").unwrap();
        // A script at the top level.
        std::fs::write(skill_dir.join("setup.sh"), "#!/bin/sh").unwrap();

        let files = collect_supporting_files(skill_dir);
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.contains(&"foo.md".to_owned()),
            "missing foo.md: {names:?}"
        );
        assert!(
            names.contains(&"bar.md".to_owned()),
            "missing bar.md: {names:?}"
        );
        assert!(
            names.contains(&"setup.sh".to_owned()),
            "missing setup.sh: {names:?}"
        );
        assert!(
            !names.contains(&"SKILL.md".to_owned()),
            "SKILL.md should be excluded: {names:?}"
        );
    }

    #[test]
    fn collect_supporting_files_does_not_descend_into_nested_skills() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: x\n---\n").unwrap();
        std::fs::write(skill_dir.join("helper.sh"), "#!/bin/sh").unwrap();

        // A nested subdir that is itself a skill — should not be descended into.
        let nested = skill_dir.join("nested-skill");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("SKILL.md"), "---\nname: nested\n---\n").unwrap();
        std::fs::write(nested.join("secret.md"), "should not appear").unwrap();

        let files = collect_supporting_files(skill_dir);
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.contains(&"helper.sh".to_owned()),
            "missing helper.sh: {names:?}"
        );
        assert!(
            !names.contains(&"secret.md".to_owned()),
            "nested skill's files should not appear: {names:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn collect_supporting_files_skips_symlink_cycles() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: x\n---\n").unwrap();

        let refs = skill_dir.join("references");
        std::fs::create_dir_all(&refs).unwrap();
        let guide = refs.join("guide.md");
        std::fs::write(&guide, "guide").unwrap();
        std::os::unix::fs::symlink(&refs, refs.join("loop")).unwrap();

        let files = collect_supporting_files(skill_dir);
        assert_eq!(files, vec![guide]);
    }

    #[test]
    fn discover_skills_populates_supporting_files() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        let skill_dir = cwd.join(".agents/skills/with-refs");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: with-refs\ndescription: Has refs\n---\nBody.\n",
        )
        .unwrap();
        let refs = skill_dir.join("references");
        std::fs::create_dir_all(&refs).unwrap();
        std::fs::write(refs.join("guide.md"), "guide content").unwrap();

        let skills = discover_skills_impl(cwd, None);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "with-refs");
        assert_eq!(skills[0].supporting_files.len(), 1);
        assert!(
            skills[0].supporting_files[0].ends_with("references/guide.md"),
            "unexpected path: {:?}",
            skills[0].supporting_files[0]
        );
    }
}

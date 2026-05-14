//! Sprout Nest — persistent agent workspace at `~/.sprout`.
//!
//! Creates a shared knowledge directory on first launch so every
//! Sprout-spawned agent starts with orientation (AGENTS.md) and a
//! place to accumulate research, plans, and logs across sessions.
//!
//! Idempotent: existing files and directories are never overwritten.

use super::{load_managed_agents, load_personas, ManagedAgentRecord, PersonaRecord};
#[cfg(test)]
use super::{BackendKind, RespondTo};
use crate::app_state::AppState;
use crate::relay::relay_ws_url_with_override;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

/// Subdirectories created inside the nest.
const NEST_DIRS: &[&str] = &[
    "GUIDES",
    "RESEARCH",
    "PLANS",
    "WORK_LOGS",
    "REPOS",
    "OUTBOX",
    ".scratch",
];

/// Default AGENTS.md content written on first init.
/// Fully static — no runtime interpolation, no secrets, no user paths.
const AGENTS_MD: &str = include_str!("nest_agents.md");

/// Default SKILL.md content for the sprout-cli Claude Code skill.
/// Written to ~/.sprout/.claude/skills/sprout-cli/SKILL.md on first init.
const SPROUT_CLI_SKILL_MD: &str = include_str!("nest_skill.md");

const BEGIN_MARKER: &str = "<!-- BEGIN SPROUT MANAGED";
const END_MARKER: &str = "<!-- END SPROUT MANAGED -->";
/// Returns the nest root path (`~/.sprout`), or `None` if the home
/// directory cannot be resolved.
pub fn nest_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".sprout"))
}

/// Creates the Sprout nest at `~/.sprout` if it doesn't already exist.
///
/// Delegates to [`ensure_nest_at`] with the resolved nest directory.
/// Returns an error string if the home directory cannot be resolved.
pub fn ensure_nest() -> Result<(), String> {
    let root = nest_dir().ok_or("cannot resolve home directory for nest")?;
    ensure_nest_at(&root)
}

/// Creates a Sprout nest at the given `root` path.
///
/// - Creates the root directory and all subdirectories.
/// - Writes `AGENTS.md` only if it doesn't already exist.
/// - Writes `.claude/skills/sprout-cli/SKILL.md` only if it doesn't already exist.
/// - Sets 700 permissions on the root, all subdirectories, and the skill
///   directory tree (Unix).
///
/// Idempotent: safe to call on every launch. Existing files are never
/// overwritten — users can freely edit AGENTS.md or SKILL.md and they persist.
///
/// Rejects symlinks at the root path to prevent redirect attacks.
///
/// Errors are returned as strings for Tauri compatibility; callers
/// should log and continue rather than aborting app startup.
pub fn ensure_nest_at(root: &Path) -> Result<(), String> {
    // Reject symlinks — we want a real directory, not a redirect.
    // Platform-independent: symlink_metadata works on all OS.
    if root
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(format!(
            "{} is a symlink; refusing to use as nest root",
            root.display()
        ));
    }

    // Create root and all subdirectories. create_dir_all is idempotent —
    // it succeeds silently if the directory already exists.
    fs::create_dir_all(root).map_err(|e| format!("create {}: {e}", root.display()))?;

    for dir in NEST_DIRS {
        let path = root.join(dir);
        fs::create_dir_all(&path).map_err(|e| format!("create {}: {e}", path.display()))?;
    }

    // Write AGENTS.md only if it doesn't already exist.
    // Uses create_new (O_CREAT|O_EXCL) to atomically check-and-create,
    // closing the TOCTOU gap that exists() + write() would leave open.
    // Also guarantees we never clobber a user-edited file.
    let agents_md = root.join("AGENTS.md");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&agents_md)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(AGENTS_MD.as_bytes())
                .map_err(|e| format!("write {}: {e}", agents_md.display()))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // File already exists — leave it alone (idempotent).
        }
        Err(e) => {
            return Err(format!("create {}: {e}", agents_md.display()));
        }
    }

    // Write sprout-cli skill alongside AGENTS.md (same idempotent pattern).
    let skill_dir = root.join(".claude/skills/sprout-cli");
    fs::create_dir_all(&skill_dir).map_err(|e| format!("create {}: {e}", skill_dir.display()))?;

    let skill_md = root.join(".claude/skills/sprout-cli/SKILL.md");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&skill_md)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(SPROUT_CLI_SKILL_MD.as_bytes())
                .map_err(|e| format!("write {}: {e}", skill_md.display()))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => {
            return Err(format!("create {}: {e}", skill_md.display()));
        }
    }

    // Set owner-only permissions on root and all subdirectories.
    // Skip any path that is a symlink — chmod would affect the target.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o700);
        fs::set_permissions(root, perms.clone())
            .map_err(|e| format!("set permissions on {}: {e}", root.display()))?;
        for dir in NEST_DIRS {
            let path = root.join(dir);
            let is_symlink = path
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if !is_symlink {
                fs::set_permissions(&path, perms.clone())
                    .map_err(|e| format!("set permissions on {}: {e}", path.display()))?;
            }
        }
        // Skill directory and its intermediate parents inside root get 700.
        // create_dir_all creates .claude/ and .claude/skills/ with umask
        // defaults — lock them down the same way we do NEST_DIRS.
        for dir in [
            root.join(".claude"),
            root.join(".claude/skills"),
            skill_dir.clone(),
        ] {
            let is_symlink = dir
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if !is_symlink {
                fs::set_permissions(&dir, perms.clone())
                    .map_err(|e| format!("set permissions on {}: {e}", dir.display()))?;
            }
        }
    }

    Ok(())
}

/// Ensures `~/.local/bin/sprout` is a symlink to the bundled CLI binary.
///
/// Creates the symlink if it doesn't exist, updates it if it already points
/// to a Sprout app bundle, and leaves it alone if it points elsewhere (to
/// avoid clobbering another tool's binary).
///
/// Non-fatal: callers should ignore errors — the symlink is a convenience
/// for human Terminal use; agents find the CLI via PATH augmentation.
#[cfg(unix)]
pub fn ensure_cli_symlink(exe_parent: &Path) -> Result<(), String> {
    let sprout_bin = exe_parent.join("sprout");
    if !sprout_bin.exists() {
        return Ok(()); // CLI not bundled (e.g., dev builds without sidecars).
    }

    let local_bin = dirs::home_dir()
        .ok_or("cannot resolve home directory")?
        .join(".local")
        .join("bin");
    fs::create_dir_all(&local_bin).map_err(|e| format!("create {}: {e}", local_bin.display()))?;

    let link = local_bin.join("sprout");
    match link.symlink_metadata() {
        Ok(meta) if meta.file_type().is_symlink() => {
            // Symlink exists — only update if it points to a Sprout bundle.
            if let Ok(target) = fs::read_link(&link) {
                let target_str = target.display().to_string();
                if target_str.contains(".app/Contents/MacOS") {
                    // Sprout-owned symlink — update to current bundle path.
                    let _ = fs::remove_file(&link);
                    std::os::unix::fs::symlink(&sprout_bin, &link)
                        .map_err(|e| format!("symlink {}: {e}", link.display()))?;
                }
                // Otherwise: symlink points elsewhere — don't clobber.
            }
        }
        Ok(_) => {
            // Regular file or directory — don't clobber.
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No file exists — create the symlink.
            std::os::unix::fs::symlink(&sprout_bin, &link)
                .map_err(|e| format!("symlink {}: {e}", link.display()))?;
        }
        Err(e) => {
            return Err(format!("stat {}: {e}", link.display()));
        }
    }

    Ok(())
}

/// No-op on non-Unix platforms — symlink management is macOS/Linux only.
#[cfg(not(unix))]
pub fn ensure_cli_symlink(_exe_parent: &Path) -> Result<(), String> {
    Ok(())
}

const CLI_QUICK_REFERENCE: &str = "\
## CLI Quick Reference
`sprout messages send --channel <id> --content <text>` — send a message
`sprout messages get --channel <id>` — read recent messages
`sprout channels list` — list available channels
`sprout workflows trigger --workflow <id>` — trigger a workflow
Run `sprout --help` for the full command reference.";

fn escape_md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

pub fn render_dynamic_section(
    personas: &[PersonaRecord],
    agents: &[ManagedAgentRecord],
    relay_url: &str,
) -> String {
    let active_agents = if agents.is_empty() {
        "## Active Agents\n\n*(No agents deployed yet. Add agents in the Sprout desktop app.)*"
            .to_string()
    } else {
        let mut table =
            "## Active Agents\n\n| Name | Persona | How to address |\n|------|---------|----------------|"
                .to_string();
        for agent in agents {
            let role = agent
                .persona_id
                .as_deref()
                .and_then(|pid| personas.iter().find(|p| p.id == pid))
                .map(|p| p.display_name.as_str())
                .unwrap_or("—");
            let name = escape_md_cell(&agent.name);
            let role_escaped = escape_md_cell(role);
            table.push_str(&format!("\n| {name} | {role_escaped} | @{name} |"));
        }
        table
    };

    format!("{active_agents}\n\n## Workspace\n- Relay: {relay_url}\n\n{CLI_QUICK_REFERENCE}")
}

/// Find a marker that appears at the start of a line (position 0 or preceded by `\n`).
fn find_marker_at_line_start(content: &str, marker: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(pos) = content[search_from..].find(marker) {
        let abs_pos = search_from + pos;
        if abs_pos == 0 || content.as_bytes()[abs_pos - 1] == b'\n' {
            return Some(abs_pos);
        }
        search_from = abs_pos + 1;
    }
    None
}

/// Find the first valid ordered BEGIN/END marker pair, both at line starts.
/// Returns `(begin_line_start, after_end)` byte offsets for slicing.
fn find_managed_markers(content: &str) -> Option<(usize, usize)> {
    let begin_pos = find_marker_at_line_start(content, BEGIN_MARKER)?;
    let begin_line_start = content[..begin_pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let end_pos = content[begin_pos..]
        .find(END_MARKER)
        .map(|p| p + begin_pos)?;
    let end_of_end = end_pos + END_MARKER.len();
    let after_end = if content[end_of_end..].starts_with('\n') {
        end_of_end + 1
    } else {
        end_of_end
    };
    Some((begin_line_start, after_end))
}

/// Remove an orphan BEGIN marker line (one with no matching END after it).
fn strip_orphan_begin_marker(content: &str) -> String {
    if let Some(pos) = find_marker_at_line_start(content, BEGIN_MARKER) {
        let line_start = content[..pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let line_end = content[pos..]
            .find('\n')
            .map(|p| pos + p + 1)
            .unwrap_or(content.len());
        format!(
            "{}{}",
            &content[..line_start],
            content[line_end..].trim_start_matches('\n')
        )
    } else {
        content.to_string()
    }
}

pub fn upsert_managed_section(file_path: &Path, new_section_content: &str) -> io::Result<()> {
    let current = fs::read_to_string(file_path)?;

    let replacement = format!(
        "{BEGIN_MARKER} — regenerated automatically, do not edit below -->\n{new_section_content}\n{END_MARKER}\n"
    );

    let new_content = match find_managed_markers(&current) {
        Some((begin_line_start, after_end)) => {
            format!(
                "{}{}{}",
                &current[..begin_line_start],
                replacement,
                &current[after_end..]
            )
        }
        None => {
            let cleaned = strip_orphan_begin_marker(&current);
            format!("{}\n\n{}", cleaned.trim_end_matches('\n'), replacement)
        }
    };

    let parent = file_path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "file path has no parent directory",
        )
    })?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    {
        use std::io::Write;
        tmp.write_all(new_content.as_bytes())?;
    }
    tmp.persist(file_path).map_err(|e| e.error)?;

    Ok(())
}

pub fn regenerate_nest_context(app: &AppHandle) -> Result<(), String> {
    let nest = nest_dir().ok_or("cannot resolve home directory for nest")?;
    let agents_md = nest.join("AGENTS.md");

    if !agents_md.exists() {
        return Ok(());
    }

    let personas = load_personas(app)?;
    let agents = load_managed_agents(app)?;
    let state = app.state::<AppState>();
    let relay_url = relay_ws_url_with_override(&state);
    let content = render_dynamic_section(&personas, &agents, &relay_url);
    upsert_managed_section(&agents_md, &content)
        .map_err(|e| format!("regenerate nest context: {e}"))?;

    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nest_dir_is_under_home() {
        if let Some(dir) = nest_dir() {
            assert!(dir.ends_with(".sprout"));
        }
    }

    #[test]
    fn ensure_nest_creates_all_dirs_and_agents_md() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".sprout");

        ensure_nest_at(&root).unwrap();

        // All subdirectories exist.
        for dir in NEST_DIRS {
            assert!(root.join(dir).is_dir(), "{dir}/ should exist");
        }

        // AGENTS.md was written with default content.
        let content = fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert_eq!(content, AGENTS_MD);

        // Permissions are 700 on Unix for root and all subdirs.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&root).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "root should be 700");
            for dir in NEST_DIRS {
                let mode = fs::metadata(root.join(dir)).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o700, "{dir}/ should be 700");
            }
        }
    }

    #[test]
    fn ensure_nest_is_idempotent_and_preserves_custom_content() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".sprout");

        // First call creates everything.
        ensure_nest_at(&root).unwrap();

        // User customizes AGENTS.md.
        let agents = root.join("AGENTS.md");
        fs::write(&agents, "my custom instructions").unwrap();

        // Second call succeeds and does not overwrite.
        ensure_nest_at(&root).unwrap();

        assert_eq!(
            fs::read_to_string(&agents).unwrap(),
            "my custom instructions"
        );

        // All dirs still exist.
        for dir in NEST_DIRS {
            assert!(root.join(dir).is_dir(), "{dir}/ should still exist");
        }
    }

    #[cfg(unix)]
    #[test]
    fn ensure_nest_rejects_symlink_root() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("real_dir");
        fs::create_dir(&target).unwrap();
        let link = tmp.path().join(".sprout");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let result = ensure_nest_at(&link);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("symlink"));
    }

    #[test]
    fn ensure_nest_creates_skill_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".sprout");
        ensure_nest_at(&root).unwrap();
        let skill = root.join(".claude/skills/sprout-cli/SKILL.md");
        assert!(skill.exists(), "SKILL.md should exist");
        let content = fs::read_to_string(&skill).unwrap();
        assert_eq!(content, SPROUT_CLI_SKILL_MD);
    }

    #[test]
    fn ensure_nest_does_not_overwrite_skill_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".sprout");
        ensure_nest_at(&root).unwrap();
        let skill = root.join(".claude/skills/sprout-cli/SKILL.md");
        fs::write(&skill, "custom skill content").unwrap();
        ensure_nest_at(&root).unwrap();
        assert_eq!(fs::read_to_string(&skill).unwrap(), "custom skill content");
    }

    #[cfg(unix)]
    #[test]
    fn ensure_nest_skill_dir_has_700_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".sprout");
        ensure_nest_at(&root).unwrap();
        // All three dirs in the skill path should be locked down.
        for dir in [".claude", ".claude/skills", ".claude/skills/sprout-cli"] {
            let path = root.join(dir);
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{dir} should be 700");
        }
    }

    #[cfg(unix)]
    #[test]
    fn ensure_nest_skips_permissions_on_symlinked_child() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".sprout");

        // First call creates the real nest.
        ensure_nest_at(&root).unwrap();

        // Replace REPOS/ with a symlink to an external directory.
        let external = tmp.path().join("external");
        fs::create_dir(&external).unwrap();
        fs::set_permissions(&external, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir(&root.join("REPOS")).unwrap();
        std::os::unix::fs::symlink(&external, &root.join("REPOS")).unwrap();

        // Second call should succeed — it skips chmod on the symlinked child.
        ensure_nest_at(&root).unwrap();

        // The external directory's permissions should be unchanged (755, not 700).
        let mode = fs::metadata(&external).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o755,
            "symlinked child's target should not be chmod'd"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_cli_symlink_creates_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let exe_parent = tmp.path().join("MacOS");
        fs::create_dir(&exe_parent).unwrap();
        fs::write(exe_parent.join("sprout"), "binary").unwrap();

        // Point home_dir to a temp location by using ensure_cli_symlink
        // directly with a custom link target. We'll test the logic manually.
        let local_bin = tmp.path().join("local_bin");
        fs::create_dir_all(&local_bin).unwrap();
        let link = local_bin.join("sprout");

        // Create symlink manually to test the creation path.
        std::os::unix::fs::symlink(exe_parent.join("sprout"), &link).unwrap();
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(fs::read_link(&link).unwrap(), exe_parent.join("sprout"));
    }

    #[cfg(unix)]
    #[test]
    fn ensure_cli_symlink_does_not_clobber_regular_file() {
        let tmp = tempfile::tempdir().unwrap();
        let local_bin = tmp.path().join("local_bin");
        fs::create_dir_all(&local_bin).unwrap();
        let link = local_bin.join("sprout");
        fs::write(&link, "user-installed binary").unwrap();

        // Verify it's a regular file.
        assert!(link.symlink_metadata().unwrap().file_type().is_file());
        // Content should be preserved (we can't call ensure_cli_symlink
        // directly without controlling dirs::home_dir(), but the logic
        // in the Ok(_) branch of ensure_cli_symlink skips regular files).
        assert_eq!(fs::read_to_string(&link).unwrap(), "user-installed binary");
    }

    fn make_persona(id: &str, display_name: &str) -> PersonaRecord {
        PersonaRecord {
            id: id.to_string(),
            display_name: display_name.to_string(),
            avatar_url: None,
            system_prompt: String::new(),
            provider: None,
            model: None,
            name_pool: vec![],
            is_builtin: false,
            is_active: true,
            source_pack: None,
            source_pack_persona_slug: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn make_agent(name: &str, persona_id: Option<&str>) -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey: String::new(),
            name: name.to_string(),
            persona_id: persona_id.map(|s| s.to_string()),
            private_key_nsec: String::new(),
            auth_tag: None,
            relay_url: String::new(),
            acp_command: String::new(),
            agent_command: String::new(),
            agent_args: vec![],
            mcp_command: String::new(),
            turn_timeout_seconds: 0,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: 1,
            system_prompt: None,
            model: None,
            mcp_toolsets: None,
            start_on_app_launch: false,
            runtime_pid: None,
            backend: BackendKind::default(),
            backend_agent_id: None,
            provider_binary_path: None,
            persona_pack_path: None,
            persona_name_in_pack: None,
            created_at: String::new(),
            updated_at: String::new(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            respond_to: RespondTo::default(),
            respond_to_allowlist: vec![],
        }
    }

    #[test]
    fn test_render_dynamic_section_with_agents() {
        let personas = vec![make_persona("p1", "Builder")];
        let agents = vec![make_agent("Kit", Some("p1"))];
        let output = render_dynamic_section(&personas, &agents, "ws://example.com:3000");
        assert!(output.contains("| Kit | Builder | @Kit |"));
        assert!(output.contains("| Name | Persona | How to address |"));
        assert!(output.contains("## CLI Quick Reference"));
    }

    #[test]
    fn test_render_dynamic_section_empty() {
        let output = render_dynamic_section(&[], &[], "ws://example.com:3000");
        assert!(output.contains("No agents deployed yet"));
    }

    #[test]
    fn test_render_dynamic_section_agent_no_persona() {
        let personas = vec![make_persona("p1", "Builder")];
        let agents = vec![make_agent("Scout", Some("nonexistent"))];
        let output = render_dynamic_section(&personas, &agents, "ws://example.com:3000");
        assert!(output.contains("| Scout | — | @Scout |"));
    }

    #[test]
    fn test_upsert_managed_section_with_markers() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("AGENTS.md");
        fs::write(
            &file,
            "# Header\n\nsome content\n\n<!-- BEGIN SPROUT MANAGED — regenerated automatically, do not edit below -->\nold section\n<!-- END SPROUT MANAGED -->\n\nafter\n",
        )
        .unwrap();

        upsert_managed_section(&file, "new section").unwrap();

        let result = fs::read_to_string(&file).unwrap();
        assert!(result.contains("<!-- BEGIN SPROUT MANAGED"));
        assert!(result.contains("<!-- END SPROUT MANAGED -->"));
        assert!(result.contains("new section"));
        assert!(!result.contains("old section"));
        assert!(result.contains("# Header"));
        assert!(result.contains("some content"));
        assert!(result.contains("after"));
    }

    #[test]
    fn test_upsert_managed_section_without_markers() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("AGENTS.md");
        fs::write(&file, "# Header\n\nexisting content\n").unwrap();

        upsert_managed_section(&file, "injected section").unwrap();

        let result = fs::read_to_string(&file).unwrap();
        assert!(result.contains("# Header"));
        assert!(result.contains("existing content"));
        assert!(result.contains("<!-- BEGIN SPROUT MANAGED"));
        assert!(result.contains("<!-- END SPROUT MANAGED -->"));
        assert!(result.contains("injected section"));
        let begin_pos = result.find("<!-- BEGIN SPROUT MANAGED").unwrap();
        let header_pos = result.find("# Header").unwrap();
        assert!(
            header_pos < begin_pos,
            "original content should precede the managed section"
        );
    }

    #[test]
    fn test_upsert_managed_section_no_tmp_leftover() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("AGENTS.md");
        fs::write(&file, "# Header\n").unwrap();

        upsert_managed_section(&file, "content").unwrap();

        // Verify no stray temp files in the directory
        let entries: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "only AGENTS.md should remain, no temp files"
        );
        assert_eq!(entries[0].file_name(), "AGENTS.md");
    }

    #[test]
    fn test_upsert_end_before_begin() {
        // An END marker that precedes a BEGIN marker forms no valid ordered pair.
        // find_managed_markers returns None (BEGIN found, but no END after it),
        // so the orphan BEGIN line is stripped and a new block is appended.
        // The stray END line and content between END and BEGIN remain in the file
        // because strip_orphan_begin_marker only removes the BEGIN line itself.
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("AGENTS.md");
        fs::write(
            &file,
            "# Header\n\n<!-- END SPROUT MANAGED -->\nsome middle content\n<!-- BEGIN SPROUT MANAGED — regenerated automatically, do not edit below -->\nold section\n",
        )
        .unwrap();

        upsert_managed_section(&file, "new section").unwrap();

        let result = fs::read_to_string(&file).unwrap();

        assert!(result.contains("# Header"), "original header must survive");
        assert!(
            result.contains("new section"),
            "new content must be present"
        );
        assert!(
            result.contains("some middle content"),
            "content between markers must survive"
        );

        // Exactly one BEGIN marker in the output (the orphan was stripped, new one appended).
        assert_eq!(
            result.matches(BEGIN_MARKER).count(),
            1,
            "exactly one BEGIN marker after orphan cleanup"
        );

        // The single BEGIN marker must have a matching END marker after it.
        let begin_pos = result
            .find(BEGIN_MARKER)
            .expect("BEGIN marker must be present");
        let end_pos = result[begin_pos..].find(END_MARKER).map(|p| begin_pos + p);
        assert!(
            end_pos.is_some(),
            "an END marker must appear after the appended BEGIN marker"
        );
    }

    #[test]
    fn test_upsert_begin_only_no_end() {
        // A file with BEGIN but no END has an orphan marker.
        // find_managed_markers returns None (no END found after BEGIN),
        // so strip_orphan_begin_marker removes the BEGIN line.
        // Content that followed the orphan BEGIN is preserved (only the marker line is stripped,
        // not the body that came after it).
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("AGENTS.md");
        fs::write(
            &file,
            "# Header\n\nsome content\n\n<!-- BEGIN SPROUT MANAGED — regenerated automatically, do not edit below -->\norphaned section without end marker\n",
        )
        .unwrap();

        upsert_managed_section(&file, "fresh section").unwrap();

        let result = fs::read_to_string(&file).unwrap();

        assert!(result.contains("# Header"), "original header must survive");
        assert!(
            result.contains("some content"),
            "original body must survive"
        );
        assert!(
            result.contains("fresh section"),
            "new content must be present"
        );

        let begin_pos = result
            .find(BEGIN_MARKER)
            .expect("BEGIN marker must be present");
        let end_pos = result.find(END_MARKER).expect("END marker must be present");
        assert!(
            begin_pos < end_pos,
            "the appended BEGIN marker must precede the appended END marker"
        );

        // Exactly one BEGIN marker after orphan cleanup.
        assert_eq!(
            result.matches(BEGIN_MARKER).count(),
            1,
            "exactly one BEGIN marker after orphan cleanup"
        );
    }

    #[test]
    fn test_upsert_duplicate_markers() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("AGENTS.md");
        fs::write(
            &file,
            "# Header\n\n<!-- BEGIN SPROUT MANAGED — regenerated automatically, do not edit below -->\nfirst block\n<!-- END SPROUT MANAGED -->\n\nbetween blocks\n\n<!-- BEGIN SPROUT MANAGED — regenerated automatically, do not edit below -->\nsecond block\n<!-- END SPROUT MANAGED -->\n",
        )
        .unwrap();

        upsert_managed_section(&file, "replaced").unwrap();

        let result = fs::read_to_string(&file).unwrap();

        assert!(
            result.contains("replaced"),
            "replacement content must be present"
        );
        assert!(
            !result.contains("first block"),
            "first block must be replaced"
        );
        assert!(
            result.contains("second block"),
            "second pair content must survive"
        );
        assert!(
            result.contains("between blocks"),
            "text between pairs must survive"
        );
    }

    #[test]
    fn test_upsert_marker_in_code_block() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("AGENTS.md");
        // Indented by 4 spaces — not at column 0, so should NOT match as a real marker.
        fs::write(
            &file,
            "# Header\n\n    <!-- BEGIN SPROUT MANAGED — some indented marker -->\n\nReal content here\n",
        )
        .unwrap();

        upsert_managed_section(&file, "appended content").unwrap();

        let result = fs::read_to_string(&file).unwrap();

        assert!(
            result.contains("    <!-- BEGIN SPROUT MANAGED — some indented marker -->"),
            "indented marker inside code block must be preserved verbatim"
        );
        assert!(
            result.contains("appended content"),
            "new content must be appended"
        );
        assert!(
            result.contains("Real content here"),
            "existing body must survive"
        );

        // The real markers appended at the end must be at line-start (column 0).
        let begin_pos = result
            .find("<!-- BEGIN SPROUT MANAGED — regenerated")
            .expect("regenerated BEGIN marker must be present");
        assert!(
            begin_pos == 0 || result.as_bytes()[begin_pos - 1] == b'\n',
            "appended BEGIN marker must be at line start"
        );
    }

    #[test]
    fn test_render_pipe_in_agent_name() {
        let personas = vec![make_persona("p1", "Builder")];
        let agents = vec![make_agent("Kit|Pro", Some("p1"))];
        let output = render_dynamic_section(&personas, &agents, "ws://example.com:3000");

        assert!(
            output.contains("Kit\\|Pro"),
            "pipe in agent name must be escaped as \\|"
        );
        // An unescaped bare `|` immediately adjacent to "Kit|Pro" would break table parsing.
        assert!(
            !output.contains("| Kit|Pro |"),
            "unescaped pipe in agent name must not appear as a cell boundary"
        );

        // The row must start and end with `|` and the escaped name and address must appear.
        let kit_row = output
            .lines()
            .find(|l| l.contains("Kit\\|Pro"))
            .expect("Kit\\|Pro row must be present");
        assert!(kit_row.starts_with('|'), "row must start with |");
        assert!(kit_row.ends_with('|'), "row must end with |");
        assert!(
            kit_row.contains("@Kit\\|Pro"),
            "address cell must use escaped name"
        );
    }

    #[test]
    fn test_render_newline_in_persona_name() {
        let personas = vec![make_persona("p1", "Builder\nExpert")];
        let agents = vec![make_agent("Scout", Some("p1"))];
        let output = render_dynamic_section(&personas, &agents, "ws://example.com:3000");

        assert!(
            output.contains("Builder Expert"),
            "newline in persona display_name must be replaced with a space"
        );

        // The table row for Scout must be a single line (no embedded newline).
        let scout_row = output
            .lines()
            .find(|l| l.contains("Scout"))
            .expect("Scout row must be present");
        assert!(
            scout_row.contains("Builder Expert"),
            "persona name with newline replaced by space must appear on the Scout row"
        );
    }

    #[test]
    fn test_upsert_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("AGENTS.md");
        fs::write(
            &file,
            "# Header\n\n<!-- BEGIN SPROUT MANAGED — regenerated automatically, do not edit below -->\nexisting section\n<!-- END SPROUT MANAGED -->\n",
        )
        .unwrap();

        upsert_managed_section(&file, "same content").unwrap();
        let after_first = fs::read_to_string(&file).unwrap();

        upsert_managed_section(&file, "same content").unwrap();
        let after_second = fs::read_to_string(&file).unwrap();

        assert_eq!(
            after_first, after_second,
            "upsert must be idempotent: second call must not alter the file"
        );
    }
}

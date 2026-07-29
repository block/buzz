//! Slack-parity chat targeting: inline `key=value` options, light NL cues,
//! and nest `environments.toml` resolution for `[Run Options]` prompt injection.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// A named multi-repo environment from `{cwd}/.buzz/environments.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct NestEnvironment {
    pub name: String,
    pub repos: Vec<String>,
    #[serde(default)]
    pub default_branch: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct EnvironmentsFile {
    #[serde(default)]
    environment: Vec<NestEnvironment>,
}

/// Parsed targeting for a turn — explicit mention options + rule defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedTargeting {
    pub repo: Option<String>,
    pub env: Option<String>,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub autopr: Option<bool>,
    pub channel: Option<String>,
    /// Repo names in scope (from env manifest and/or explicit `repo=`).
    pub repos: Vec<String>,
    /// Absolute `{cwd}/REPOS/{name}` paths that exist on disk.
    pub preferred_repo_paths: Vec<String>,
}

impl ResolvedTargeting {
    fn is_empty(&self) -> bool {
        self.repo.is_none()
            && self.env.is_none()
            && self.branch.is_none()
            && self.model.is_none()
            && self.autopr.is_none()
            && self.channel.is_none()
            && self.repos.is_empty()
            && self.preferred_repo_paths.is_empty()
    }
}

/// Load nest environments from `{cwd}/.buzz/environments.toml`.
///
/// Missing or unreadable files yield an empty list (never an error).
pub fn load_environments(nest_cwd: &str) -> Vec<NestEnvironment> {
    let path = Path::new(nest_cwd).join(".buzz").join("environments.toml");
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    match toml::from_str::<EnvironmentsFile>(&contents) {
        Ok(file) => file.environment,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "failed to parse environments.toml — ignoring"
            );
            Vec::new()
        }
    }
}

/// Parse inline `key=value` options and light NL cues from mention text.
///
/// Supported keys: `repo`, `env`/`environment`, `branch`, `model`, `autopr`,
/// `channel`. Values may be bare tokens or double/single-quoted (spaces OK).
fn parse_mention_options(content: &str) -> ResolvedTargeting {
    let mut out = ResolvedTargeting::default();
    let mut rest = content;

    while let Some((key, value, next)) = take_next_option(rest) {
        apply_option(&mut out, &key, &value);
        rest = next;
    }

    // Natural language fills only fields not already set by key=value.
    if out.env.is_none() {
        if let Some(env) = parse_use_environment(content) {
            out.env = Some(env);
        }
    }
    if out.repo.is_none() {
        if let Some(repo) = parse_in_repo(content) {
            out.repo = Some(repo);
        }
    }

    out
}

fn apply_option(out: &mut ResolvedTargeting, key: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    match key {
        "repo" => out.repo = Some(value.to_string()),
        "env" | "environment" => out.env = Some(value.to_string()),
        "branch" => out.branch = Some(value.to_string()),
        "model" => out.model = Some(value.to_string()),
        "autopr" => {
            out.autopr = match value.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" => Some(true),
                "false" | "0" | "no" => Some(false),
                _ => None,
            };
        }
        "channel" => out.channel = Some(value.to_string()),
        _ => {}
    }
}

/// Find the next `key=value` / `key="value"` occurrence in `s`.
///
/// Returns `(key, value, remainder_after_match)`.
fn take_next_option(s: &str) -> Option<(String, String, &str)> {
    // Scan for a known key followed by `=`.
    const KEYS: &[&str] = &[
        "environment",
        "repo",
        "env",
        "branch",
        "model",
        "autopr",
        "channel",
    ];

    let lower = s.to_ascii_lowercase();
    let mut best: Option<(usize, &str)> = None;
    for key in KEYS {
        let mut search_from = 0;
        while let Some(rel) = lower[search_from..].find(key) {
            let start = search_from + rel;
            let after_key = start + key.len();
            // Boundary: start of string or non-identifier char before key.
            let ok_before = start == 0
                || !s.as_bytes()[start - 1].is_ascii_alphanumeric()
                    && s.as_bytes()[start - 1] != b'_';
            if ok_before && lower.get(after_key..after_key + 1) == Some("=") {
                match best {
                    Some((best_start, _)) if best_start <= start => {}
                    _ => best = Some((start, key)),
                }
                break;
            }
            search_from = start + 1;
        }
    }

    let (start, key) = best?;
    let value_start = start + key.len() + 1; // skip `key=`
    let (value, value_end) = parse_option_value(&s[value_start..])?;
    let abs_end = value_start + value_end;
    Some((key.to_string(), value, &s[abs_end..]))
}

/// Parse a value after `=`: quoted string or bare token.
///
/// Returns `(value, bytes_consumed_from_input)`.
fn parse_option_value(s: &str) -> Option<(String, usize)> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    match bytes[0] {
        b'"' | b'\'' => {
            let quote = bytes[0];
            let mut i = 1;
            while i < bytes.len() {
                if bytes[i] == quote {
                    let value = s[1..i].to_string();
                    return Some((value, i + 1));
                }
                // Allow simple backslash-escape of the quote.
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                i += 1;
            }
            None
        }
        _ => {
            let end = s
                .find(|c: char| c.is_whitespace() || c == ',')
                .unwrap_or(s.len());
            if end == 0 {
                return None;
            }
            Some((s[..end].to_string(), end))
        }
    }
}

/// `use the X environment` / `use the "X Y" environment`.
fn parse_use_environment(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let marker = "use the ";
    let mut search_from = 0;
    while let Some(rel) = lower[search_from..].find(marker) {
        let start = search_from + rel + marker.len();
        let after = &content[start..];
        let (name, name_end) = match after.as_bytes().first() {
            Some(b'"') | Some(b'\'') => parse_option_value(after)?,
            _ => {
                // Unquoted: take tokens until ` environment`.
                let env_word = " environment";
                let after_lower = after.to_ascii_lowercase();
                let end = after_lower.find(env_word)?;
                let name = after[..end].trim();
                if name.is_empty() {
                    search_from = start + 1;
                    continue;
                }
                (name.to_string(), end)
            }
        };
        let after_name = &after[name_end..];
        if after_name
            .to_ascii_lowercase()
            .trim_start()
            .starts_with("environment")
        {
            let name = name.trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
        search_from = start + 1;
    }
    None
}

/// Stopwords that must not match as `in <repo>` (single-token, no spaces).
const IN_REPO_STOPWORDS: &[&str] = &[
    "a", "an", "the", "this", "that", "these", "those", "my", "our", "your", "their", "its", "his",
    "her", "order", "fact", "case", "progress", "general", "particular", "question", "mind",
    "practice", "addition", "short", "summary", "turn", "channel", "thread", "reply", "here",
    "there", "place", "time", "front", "back", "parallel", "series", "production", "staging",
    "development", "code", "chat", "scope", "context",
];

/// `in <reponame>` — single token, no spaces; avoid common English stopwords.
fn parse_in_repo(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(rel) = lower[search_from..].find(" in ") {
        let start = search_from + rel + " in ".len();
        let after = &content[start..];
        let token_end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
            .unwrap_or(after.len());
        if token_end == 0 {
            search_from = start;
            continue;
        }
        let token = &after[..token_end];
        let token_lower = token.to_ascii_lowercase();
        if IN_REPO_STOPWORDS.contains(&token_lower.as_str()) {
            search_from = start;
            continue;
        }
        // Require at least one alphanumeric so punctuation-only tokens drop.
        if !token.bytes().any(|b| b.is_ascii_alphanumeric()) {
            search_from = start;
            continue;
        }
        return Some(token.to_string());
    }
    // Also allow message starting with `in <repo>` (case-insensitive).
    let trimmed = content.trim_start();
    if trimmed.len() >= 3 && trimmed[..2].eq_ignore_ascii_case("in") && trimmed.as_bytes()[2] == b' '
    {
        let rest = &trimmed[3..];
        let token_end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
            .unwrap_or(rest.len());
        if token_end > 0 {
            let token = &rest[..token_end];
            let token_lower = token.to_ascii_lowercase();
            if !IN_REPO_STOPWORDS.contains(&token_lower.as_str())
                && token.bytes().any(|b| b.is_ascii_alphanumeric())
            {
                return Some(token.to_string());
            }
        }
    }
    None
}

/// Resolve run targeting for a turn.
///
/// Explicit inline / NL options from `content` win over `rule_target_*`.
/// When an env is set, repos come from `{nest_cwd}/.buzz/environments.toml`.
/// Preferred paths are `{nest_cwd}/REPOS/{name}` when that directory exists.
pub fn resolve_run_targeting(
    content: &str,
    rule_target_repo: Option<&str>,
    rule_target_env: Option<&str>,
    nest_cwd: Option<&str>,
) -> ResolvedTargeting {
    let mut parsed = parse_mention_options(content);

    if parsed.repo.is_none() {
        if let Some(r) = rule_target_repo.map(str::trim).filter(|s| !s.is_empty()) {
            parsed.repo = Some(r.to_string());
        }
    }
    if parsed.env.is_none() {
        if let Some(e) = rule_target_env.map(str::trim).filter(|s| !s.is_empty()) {
            parsed.env = Some(e.to_string());
        }
    }

    let cwd = nest_cwd.map(str::trim).filter(|s| !s.is_empty());
    let envs = cwd.map(load_environments).unwrap_or_default();

    if let Some(ref env_name) = parsed.env {
        if let Some(entry) = envs
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(env_name))
        {
            // Normalize to the manifest spelling.
            parsed.env = Some(entry.name.clone());
            parsed.repos = entry.repos.clone();
            if parsed.branch.is_none() {
                if let Some(ref b) = entry.default_branch {
                    parsed.branch = Some(b.clone());
                }
            }
        }
    }

    // Explicit repo is always in scope even without a matching env.
    if let Some(ref repo) = parsed.repo {
        if !parsed
            .repos
            .iter()
            .any(|r| r.eq_ignore_ascii_case(repo))
        {
            parsed.repos.push(repo.clone());
        }
    }

    if let Some(cwd) = cwd {
        let mut paths = Vec::new();
        for name in &parsed.repos {
            let path = PathBuf::from(cwd).join("REPOS").join(name);
            if path.is_dir() {
                paths.push(path.display().to_string());
            }
        }
        // Explicit repo preferred path even when not yet in repos list (edge).
        if let Some(ref repo) = parsed.repo {
            let path = PathBuf::from(cwd).join("REPOS").join(repo);
            if path.is_dir() {
                let display = path.display().to_string();
                if !paths.iter().any(|p| p == &display) {
                    paths.insert(0, display);
                }
            }
        }
        parsed.preferred_repo_paths = paths;
    }

    parsed
}

/// Render a `[Run Options]` prompt section, or `None` when nothing is set.
pub fn format_run_options_section(targeting: &ResolvedTargeting) -> Option<String> {
    if targeting.is_empty() {
        return None;
    }

    let mut lines = vec!["[Run Options]".to_string()];
    if let Some(ref env) = targeting.env {
        lines.push(format!("env: {env}"));
    }
    if let Some(ref repo) = targeting.repo {
        lines.push(format!("repo: {repo}"));
    }
    if !targeting.repos.is_empty() {
        lines.push(format!("repos: {}", targeting.repos.join(", ")));
    }
    if let Some(ref branch) = targeting.branch {
        lines.push(format!("branch: {branch}"));
    }
    if let Some(ref model) = targeting.model {
        lines.push(format!("model: {model}"));
    }
    if let Some(autopr) = targeting.autopr {
        lines.push(format!("autopr: {autopr}"));
    }
    if let Some(ref channel) = targeting.channel {
        lines.push(format!("channel: {channel}"));
    }
    if !targeting.preferred_repo_paths.is_empty() {
        lines.push("preferred repo paths:".to_string());
        for p in &targeting.preferred_repo_paths {
            lines.push(format!("- {p}"));
        }
    }

    lines.push(
        "Prefer these checkouts for this turn when present. Honor branch / model / autopr / \
         channel when set (autopr=false means do not auto-open a PR; channel= means post \
         updates there when membership allows)."
            .to_string(),
    );
    lines.push(
        "Handoff: on completion include the PR URL (with buzz pr --channel) and the absolute \
         worktree path so a human can Open in Cursor Desktop."
            .to_string(),
    );

    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_nest() -> tempfile_dir::TmpNest {
        tempfile_dir::TmpNest::new()
    }

    /// Minimal temp-dir helper so tests don't need the `tempfile` crate.
    mod tempfile_dir {
        use std::path::PathBuf;

        pub struct TmpNest {
            pub path: PathBuf,
        }

        impl TmpNest {
            pub fn new() -> Self {
                let path = std::env::temp_dir().join(format!(
                    "buzz-acp-options-test-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                std::fs::create_dir_all(path.join(".buzz")).unwrap();
                std::fs::create_dir_all(path.join("REPOS")).unwrap();
                Self { path }
            }

            pub fn path_str(&self) -> &str {
                self.path.to_str().unwrap()
            }
        }

        impl Drop for TmpNest {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }
    }

    #[test]
    fn parses_quoted_env_and_autopr_false() {
        let t = parse_mention_options(
            r#"@Cursor env="Platform Staging" branch=dev autopr=false fix auth"#,
        );
        assert_eq!(t.env.as_deref(), Some("Platform Staging"));
        assert_eq!(t.branch.as_deref(), Some("dev"));
        assert_eq!(t.autopr, Some(false));
    }

    #[test]
    fn parses_single_quoted_channel_with_spaces() {
        let t = parse_mention_options(r#"channel='eng platform' repo=web"#);
        assert_eq!(t.channel.as_deref(), Some("eng platform"));
        assert_eq!(t.repo.as_deref(), Some("web"));
    }

    #[test]
    fn parses_use_the_environment_nl() {
        let t = parse_mention_options(r#"please use the "Platform Staging" environment for this"#);
        assert_eq!(t.env.as_deref(), Some("Platform Staging"));

        let t2 = parse_mention_options("use the Platform environment please");
        assert_eq!(t2.env.as_deref(), Some("Platform"));
    }

    #[test]
    fn parses_in_repo_without_overmatching() {
        let t = parse_mention_options("fix the bug in web please");
        assert_eq!(t.repo.as_deref(), Some("web"));

        let t2 = parse_mention_options("do this in the meantime");
        assert!(t2.repo.is_none(), "must not match stopword 'the'");

        let t3 = parse_mention_options("keep work in progress");
        assert!(t3.repo.is_none(), "must not match stopword 'progress'");
    }

    #[test]
    fn explicit_inline_wins_over_rule_targets() {
        let nest = tmp_nest();
        let resolved = resolve_run_targeting(
            "repo=web env=FromChat",
            Some("rule-repo"),
            Some("rule-env"),
            Some(nest.path_str()),
        );
        assert_eq!(resolved.repo.as_deref(), Some("web"));
        assert_eq!(resolved.env.as_deref(), Some("FromChat"));
    }

    #[test]
    fn rule_targets_apply_when_content_has_none() {
        let nest = tmp_nest();
        let resolved =
            resolve_run_targeting("please fix auth", Some("api"), Some("Platform"), Some(nest.path_str()));
        assert_eq!(resolved.repo.as_deref(), Some("api"));
        assert_eq!(resolved.env.as_deref(), Some("Platform"));
    }

    #[test]
    fn loads_environments_toml_and_resolves_paths() {
        let nest = tmp_nest();
        let cwd = nest.path_str();
        fs::write(
            nest.path.join(".buzz/environments.toml"),
            r#"
[[environment]]
name = "Platform"
repos = ["web", "api"]
default_branch = "main"
"#,
        )
        .unwrap();
        fs::create_dir_all(nest.path.join("REPOS/web")).unwrap();
        fs::create_dir_all(nest.path.join("REPOS/api")).unwrap();

        let envs = load_environments(cwd);
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].name, "Platform");
        assert_eq!(envs[0].repos, vec!["web", "api"]);
        assert_eq!(envs[0].default_branch.as_deref(), Some("main"));

        let resolved = resolve_run_targeting("env=Platform", None, None, Some(cwd));
        assert_eq!(resolved.env.as_deref(), Some("Platform"));
        assert_eq!(resolved.repos, vec!["web", "api"]);
        assert_eq!(resolved.branch.as_deref(), Some("main"));
        assert_eq!(resolved.preferred_repo_paths.len(), 2);
        assert!(resolved
            .preferred_repo_paths
            .iter()
            .any(|p| p.ends_with("REPOS/web")));
        assert!(resolved
            .preferred_repo_paths
            .iter()
            .any(|p| p.ends_with("REPOS/api")));
    }

    #[test]
    fn missing_environments_file_is_empty() {
        let nest = tmp_nest();
        let envs = load_environments(nest.path_str());
        assert!(envs.is_empty());
    }

    #[test]
    fn preferred_path_for_explicit_repo_when_dir_exists() {
        let nest = tmp_nest();
        fs::create_dir_all(nest.path.join("REPOS/buzz")).unwrap();
        let resolved = resolve_run_targeting("repo=buzz", None, None, Some(nest.path_str()));
        assert_eq!(resolved.repo.as_deref(), Some("buzz"));
        assert_eq!(resolved.preferred_repo_paths.len(), 1);
        assert!(resolved.preferred_repo_paths[0].ends_with("REPOS/buzz"));
    }

    #[test]
    fn format_run_options_none_when_empty() {
        assert!(format_run_options_section(&ResolvedTargeting::default()).is_none());
    }

    #[test]
    fn format_run_options_lists_set_fields() {
        let section = format_run_options_section(&ResolvedTargeting {
            repo: Some("web".into()),
            env: Some("Platform".into()),
            branch: Some("dev".into()),
            model: None,
            autopr: Some(false),
            channel: None,
            repos: vec!["web".into(), "api".into()],
            preferred_repo_paths: vec!["/nest/REPOS/web".into()],
        })
        .unwrap();
        assert!(section.starts_with("[Run Options]"));
        assert!(section.contains("env: Platform"));
        assert!(section.contains("repo: web"));
        assert!(section.contains("autopr: false"));
        assert!(section.contains("- /nest/REPOS/web"));
        assert!(section.contains("Handoff:"));
        assert!(section.contains("Open in Cursor"));
    }
}

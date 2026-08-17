//! GitHub skill import (ported from claude-code-cli-ui import workflow).

use crate::events::AgentSkillImported;

/// Parsed GitHub repository reference from user input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubRepoRef {
    /// `owner/repo` slug.
    pub slug: String,
    /// Optional branch or tag.
    pub r#ref: Option<String>,
}

/// Planned skill import before event emission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillImportPlan {
    /// Target skill id (directory slug).
    pub skill_id: String,
    /// Source repository.
    pub repo: GithubRepoRef,
    /// Optional subdirectory within the repo.
    pub path: Option<String>,
}

/// Errors during skill import planning.
#[derive(Debug, thiserror::Error)]
pub enum SkillImportError {
    /// URL or slug could not be parsed.
    #[error("invalid GitHub URL: {0}")]
    InvalidUrl(String),
    /// Skill id failed validation.
    #[error("invalid skill id: {0}")]
    InvalidSkillId(String),
}

/// Parse `https://github.com/owner/repo` or `owner/repo` into a repo ref.
pub fn parse_github_repo(input: &str) -> Result<GithubRepoRef, SkillImportError> {
    let trimmed = input.trim().trim_end_matches('/');
    let slug = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
        .unwrap_or(trimmed);
    let parts: Vec<&str> = slug.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return Err(SkillImportError::InvalidUrl(input.to_string()));
    }
    Ok(GithubRepoRef {
        slug: format!("{}/{}", parts[0], parts[1]),
        r#ref: parts.get(2).map(|s| s.to_string()),
    })
}

/// Build an import plan from repo URL and skill slug.
pub fn plan_skill_import(
    repo_input: &str,
    skill_id: &str,
    path: Option<&str>,
) -> Result<SkillImportPlan, SkillImportError> {
    if skill_id.is_empty()
        || !skill_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(SkillImportError::InvalidSkillId(skill_id.to_string()));
    }
    Ok(SkillImportPlan {
        skill_id: skill_id.to_string(),
        repo: parse_github_repo(repo_input)?,
        path: path.map(str::to_string),
    })
}

/// Convert a successful import plan to a Nostr event payload (kind 47250).
pub fn import_plan_to_event(plan: &SkillImportPlan, commit: Option<&str>) -> AgentSkillImported {
    AgentSkillImported {
        skill_id: plan.skill_id.clone(),
        source_repo: Some(format!("https://github.com/{}", plan.repo.slug)),
        source_commit: commit.map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_github_https() {
        let repo = parse_github_repo("https://github.com/block/buzz").expect("parse");
        assert_eq!(repo.slug, "block/buzz");
    }

    #[test]
    fn plan_rejects_bad_skill_id() {
        assert!(plan_skill_import("block/buzz", "../bad", None).is_err());
    }
}

use buzz_core::agent_skill::{SkillTestV1, SkillVersionV1};
use sha2::{Digest, Sha256};

const PROHIBITED_PATTERNS: &[&str] = &[
    "api key",
    "password",
    "new endpoint",
    "install model",
    "change provider",
    "cloud fallback",
    "disable security",
    "release configuration",
    "automatic external action",
    "autonomous external action",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EvaluationReport {
    pub evaluation_id: String,
    pub check_ids: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum EvaluationError {
    #[error("candidate skill failed deterministic evaluation")]
    Rejected,
}

pub(crate) fn evaluate_candidate(
    candidate: &SkillVersionV1,
    parent: Option<&SkillVersionV1>,
) -> Result<EvaluationReport, EvaluationError> {
    candidate
        .validate()
        .map_err(|_| EvaluationError::Rejected)?;
    if !has_exact_frontmatter_name(&candidate.skill_md, &candidate.skill_id)
        || contains_prohibited_pattern(&candidate.skill_md)
        || !tests_pass(candidate)
        || !inherits_parent_checks(candidate, parent)
    {
        return Err(EvaluationError::Rejected);
    }

    let check_ids = vec![
        "contract-valid".to_string(),
        "frontmatter-name".to_string(),
        "inherited-tests".to_string(),
        "policy-boundary".to_string(),
        "replay-checks".to_string(),
    ];
    let seed = format!(
        "{}\0{}\0{}",
        candidate.version_id,
        candidate.content_hash,
        check_ids.join("\0")
    );
    Ok(EvaluationReport {
        evaluation_id: format!(
            "evaluation-{}",
            hex::encode(Sha256::digest(seed.as_bytes()))
        ),
        check_ids,
    })
}

fn has_exact_frontmatter_name(body: &str, skill_id: &str) -> bool {
    let mut lines = body.lines();
    if lines.next() != Some("---") {
        return false;
    }
    let expected = format!("name: {skill_id}");
    let mut found = false;
    for line in lines.take(32) {
        if line == "---" {
            return found;
        }
        if line == expected {
            found = true;
        }
    }
    false
}

fn contains_prohibited_pattern(body: &str) -> bool {
    let lowercase = body.to_ascii_lowercase();
    PROHIBITED_PATTERNS
        .iter()
        .any(|pattern| lowercase.contains(pattern))
}

fn inherits_parent_checks(candidate: &SkillVersionV1, parent: Option<&SkillVersionV1>) -> bool {
    let Some(parent) = parent else {
        return candidate.parent_version_id.is_none();
    };
    if candidate.parent_version_id.as_deref() != Some(parent.version_id.as_str()) {
        return false;
    }
    let expected = parent
        .inherited_tests
        .iter()
        .chain(parent.regression_tests.iter())
        .collect::<Vec<_>>();
    let actual = candidate.inherited_tests.iter().collect::<Vec<_>>();
    expected == actual
}

fn tests_pass(candidate: &SkillVersionV1) -> bool {
    candidate
        .inherited_tests
        .iter()
        .chain(candidate.regression_tests.iter())
        .all(|check| check_passes(check, &candidate.skill_md))
}

fn check_passes(check: &SkillTestV1, body: &str) -> bool {
    match check.kind.as_str() {
        "contains" => body.contains(&check.expected),
        "not_contains" => !body.contains(&check.expected),
        "exact" => body == check.expected,
        _ => false,
    }
}

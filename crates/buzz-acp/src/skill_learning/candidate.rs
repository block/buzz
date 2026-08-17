use buzz_core::agent_skill::{skill_body_hash, SkillScope, SkillTestV1, SkillVersionV1};
use sha2::{Digest, Sha256};

const MAX_NORMALIZED_TASK_BYTES: usize = 512;

pub(super) fn normalize_task(task: &str) -> String {
    let mut normalized = String::new();
    let mut token = String::new();

    let flush = |token: &mut String, output: &mut String| {
        if token.is_empty() {
            return;
        }
        let replacement = if token.chars().any(|character| character.is_ascii_digit()) {
            "<value>"
        } else if token.len() >= 24
            && token
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
        {
            "<identifier>"
        } else {
            token.as_str()
        };
        if !output.is_empty() {
            output.push(' ');
        }
        output.push_str(replacement);
        token.clear();
    };

    for character in task.trim().to_lowercase().chars() {
        if character.is_alphanumeric() || matches!(character, '-' | '_' | '@' | '.' | ':') {
            token.push(character);
        } else {
            flush(&mut token, &mut normalized);
        }
    }
    flush(&mut token, &mut normalized);
    truncate_utf8(&mut normalized, MAX_NORMALIZED_TASK_BYTES);
    normalized
}

pub(super) fn skill_id_for_task(normalized_task: &str) -> String {
    let hash = hex::encode(Sha256::digest(normalized_task.as_bytes()));
    format!("learned-{}", &hash[..12])
}

pub(super) struct CandidateInput<'a> {
    pub normalized_task: &'a str,
    pub source_experience_ids: Vec<String>,
    pub specialist_id: &'a str,
    pub created_at: &'a str,
    pub parent: Option<&'a SkillVersionV1>,
}

pub(super) fn build_candidate(input: CandidateInput<'_>) -> SkillVersionV1 {
    let skill_id = skill_id_for_task(input.normalized_task);
    let inherited_tests: Vec<SkillTestV1> = input
        .parent
        .map(|parent| {
            parent
                .inherited_tests
                .iter()
                .chain(parent.regression_tests.iter())
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let regression_tests = if inherited_tests.is_empty() {
        vec![
            SkillTestV1 {
                check_id: "procedure-heading".to_string(),
                kind: "contains".to_string(),
                expected: "# Procedure".to_string(),
            },
            SkillTestV1 {
                check_id: "boundaries-heading".to_string(),
                kind: "contains".to_string(),
                expected: "# Boundaries".to_string(),
            },
            SkillTestV1 {
                check_id: "task-pattern".to_string(),
                kind: "contains".to_string(),
                expected: input.normalized_task.to_string(),
            },
        ]
    } else {
        vec![]
    };
    let skill_md = format!(
        "---\nname: {skill_id}\ndescription: Reusable procedure learned from repeated successful Command Adviser work.\n---\n# Repeated task pattern\n{}\n\n# Procedure\n1. Confirm the current request, due time, and missing inputs.\n2. Recall relevant active experience and retrieve applicable cited doctrine or reference evidence.\n3. Complete the work with the information available; identify missing facts and material risk without inventing status.\n4. Return the concise result, proposed action, and any follow-up needed.\n\n# Boundaries\nThis skill does not grant tools, credentials, network access, provider changes, release changes, or external-action authority.\n",
        input.normalized_task
    );
    let version_seed = format!(
        "{}\0{}\0{}\0{}",
        skill_id,
        input
            .parent
            .map(|parent| parent.version_id.as_str())
            .unwrap_or(""),
        input.source_experience_ids.join("\0"),
        skill_body_hash(&skill_md)
    );
    let version_id = format!(
        "version-{}",
        hex::encode(Sha256::digest(version_seed.as_bytes()))
    );

    SkillVersionV1 {
        skill_id,
        version_id,
        parent_version_id: input.parent.map(|parent| parent.version_id.clone()),
        scope: SkillScope::SpecialistPrivate,
        specialist_id: Some(input.specialist_id.to_string()),
        team_id: None,
        created_at: input.created_at.to_string(),
        source_experience_ids: input.source_experience_ids,
        required_tools: vec![],
        inherited_tests,
        regression_tests,
        content_hash: skill_body_hash(&skill_md),
        skill_md,
    }
}

fn truncate_utf8(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}

#[cfg(test)]
mod tests {
    use super::normalize_task;

    #[test]
    fn normalization_replaces_changing_numeric_identifiers() {
        assert_eq!(
            normalize_task("Prepare serial 4819 readiness"),
            normalize_task("Prepare serial 9921 readiness")
        );
    }
}

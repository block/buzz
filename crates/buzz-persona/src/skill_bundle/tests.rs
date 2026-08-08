use super::*;

fn skill_md(name: &str, description: &str, extra: &str) -> Vec<u8> {
    format!(
        "---\nname: {name}\ndescription: {description}\n{extra}---\n\n# Instructions\n\nInspect production health.\n"
    )
    .into_bytes()
}

fn skill(name: &str, description: &str) -> PortableSkill {
    PortableSkill {
        name: name.to_string(),
        description: description.to_string(),
        files: vec![
            PortableSkillFile {
                path: "SKILL.md".to_string(),
                bytes: skill_md(name, description, "allowed-tools: Bash(vercel:*) Read\n"),
                executable: false,
            },
            PortableSkillFile {
                path: "scripts/check.sh".to_string(),
                bytes: b"#!/bin/sh\nexit 0\n".to_vec(),
                executable: true,
            },
            PortableSkillFile {
                path: "assets/status.png".to_string(),
                bytes: vec![0, 159, 146, 150, 255],
                executable: false,
            },
            PortableSkillFile {
                path: "references/caf\u{00e9}.md".to_string(),
                bytes: Vec::new(),
                executable: false,
            },
        ],
    }
}

fn bundle(skills: Vec<PortableSkill>) -> SkillBundle {
    SkillBundle {
        schema_version: SKILL_BUNDLE_SCHEMA_VERSION,
        skills,
    }
}

#[test]
fn preserves_complete_agent_skills_directories_and_review_metadata() {
    let bundle = bundle(vec![skill(
        "production-health",
        "Inspect production health",
    )]);
    bundle.validate().unwrap();
    let inspection = bundle.inspection().unwrap();
    assert_eq!(inspection.skills.len(), 1);
    assert_eq!(
        inspection.skills[0].requested_allowed_tools.as_deref(),
        Some("Bash(vercel:*) Read")
    );
    assert_eq!(inspection.skills[0].files.len(), 4);
    assert!(inspection.skills[0]
        .files
        .iter()
        .any(|file| file.path == "scripts/check.sh" && file.executable));
    assert!(inspection.skills[0]
        .files
        .iter()
        .any(|file| file.path == "assets/status.png" && !file.is_utf8));
}

#[test]
fn digest_is_deterministic_across_skill_and_file_order() {
    let mut alpha = skill("alpha", "Alpha Skill");
    let beta = skill("beta", "Beta Skill");
    let first = bundle(vec![alpha.clone(), beta.clone()]);
    alpha.files.reverse();
    let second = bundle(vec![beta, alpha]);
    assert_eq!(
        first.canonical_digest().unwrap(),
        second.canonical_digest().unwrap()
    );
}

#[test]
fn digest_changes_with_every_identity_bearing_field() {
    let original = skill("identity", "Identity Skill");
    let original_digest = bundle(vec![original.clone()]).canonical_digest().unwrap();
    let assert_changed = |candidate: PortableSkill| {
        assert_ne!(
            bundle(vec![candidate]).canonical_digest().unwrap(),
            original_digest
        );
    };

    let mut changed = original.clone();
    changed.files[2].bytes[1] ^= 1;
    assert_changed(changed);

    let mut changed = original.clone();
    changed.files[3].path = "references/renamed.md".to_string();
    assert_changed(changed);

    let mut changed = original.clone();
    changed.files[2].executable = true;
    assert_changed(changed);

    let mut changed = original.clone();
    changed.name = "renamed".to_string();
    changed.files[0].bytes = skill_md(
        "renamed",
        "Identity Skill",
        "allowed-tools: Bash(vercel:*) Read\n",
    );
    assert_changed(changed);

    let mut changed = original;
    changed.description = "Changed description".to_string();
    changed.files[0].bytes = skill_md(
        "identity",
        "Changed description",
        "allowed-tools: Bash(vercel:*) Read\n",
    );
    assert_changed(changed);

    let inspection = bundle(vec![skill("identity", "Identity Skill")])
        .inspection()
        .unwrap();
    assert_eq!(
        inspection.skills[0]
            .files
            .iter()
            .find(|file| file.path == "assets/status.png")
            .map(|file| file.sha256.as_str()),
        Some("e92ad0d01485ac7095ffc21874b70b74f9667cf2c937b8ab2527a96d847fb21e")
    );
}

#[test]
fn rejects_missing_or_mismatched_skill_md() {
    let mut missing = skill("safe-skill", "Safe Skill");
    missing.files.retain(|file| file.path != "SKILL.md");
    assert!(bundle(vec![missing]).validate().is_err());

    let mut mismatch = skill("safe-skill", "Safe Skill");
    mismatch.files[0].bytes = skill_md("different-skill", "Safe Skill", "");
    assert!(bundle(vec![mismatch]).validate().is_err());
}

#[test]
fn preserves_valid_markdown_formatting_and_warns_for_invisible_unicode() {
    let mut ordinary = skill("ordinary-emoji", "Ordinary emoji");
    ordinary.files[0]
        .bytes
        .extend_from_slice("\nFamily: 👩\u{200d}💻\u{fe0f}.".as_bytes());
    let ordinary = bundle(vec![ordinary]);
    ordinary.validate().unwrap();
    assert!(ordinary.review_warnings().unwrap().is_empty());

    let mut candidate = skill("safe-skill", "Safe Skill");
    candidate.files[0]
        .bytes
        .extend_from_slice("\nRTL: \u{2067}مرحبا\u{2069}.".as_bytes());
    let candidate = bundle(vec![candidate]);
    candidate.validate().unwrap();
    let warnings = candidate.review_warnings().unwrap();
    assert!(warnings
        .iter()
        .any(|warning| warning.message.contains("Unicode formatting")));
}

#[test]
fn accepts_normalized_unicode_skill_names() {
    bundle(vec![skill("données", "Analyse des données")])
        .validate()
        .unwrap();

    let oversized = "𐐨".repeat(64);
    assert!(bundle(vec![skill(&oversized, "Too many path bytes")])
        .validate()
        .is_err());
}

#[test]
fn accepts_multiline_description_and_frontmatter_at_eof() {
    let description = "Inspect production health\nand explain when to intervene.\n";
    let skill = PortableSkill {
        name: "production-health".to_string(),
        description: description.to_string(),
        files: vec![PortableSkillFile {
            path: "SKILL.md".to_string(),
            bytes: b"---\nname: production-health\ndescription: |\n  Inspect production health\n  and explain when to intervene.\n---"
                .to_vec(),
            executable: false,
        }],
    };
    bundle(vec![skill]).validate().unwrap();
}

#[test]
fn rejects_traversal_windows_and_non_normalized_paths() {
    for path in [
        "../secret",
        "/secret",
        "refs\\secret",
        "C:secret",
        "CON.txt",
        "COM\u{00b9}.txt",
        "LPT\u{00b2}",
        "trailing.",
        "cafe\u{0301}.md",
        " deceptive.txt",
        "safe\u{202e}gnp.txt",
        &"a".repeat(256),
    ] {
        let mut candidate = skill("safe-skill", "Safe Skill");
        candidate.files.push(PortableSkillFile {
            path: path.to_string(),
            bytes: b"content".to_vec(),
            executable: false,
        });
        assert!(bundle(vec![candidate]).validate().is_err(), "{path}");
    }
}

#[test]
fn rejects_windows_reserved_skill_directory_names() {
    for name in ["con", "nul", "com1", "lpt9"] {
        assert!(bundle(vec![skill(name, "Reserved name")])
            .validate()
            .is_err());
    }
}

#[test]
fn rejects_casefold_and_file_directory_collisions_with_interposers() {
    let mut casefold = skill("safe-skill", "Safe Skill");
    for path in ["Guide.md", "guide.md"] {
        casefold.files.push(PortableSkillFile {
            path: path.to_string(),
            bytes: b"content".to_vec(),
            executable: false,
        });
    }
    assert!(bundle(vec![casefold]).validate().is_err());

    let mut unicode_casefold = skill("safe-skill", "Safe Skill");
    for path in ["οσ.md", "Ος.md"] {
        unicode_casefold.files.push(PortableSkillFile {
            path: path.to_string(),
            bytes: b"content".to_vec(),
            executable: false,
        });
    }
    assert!(bundle(vec![unicode_casefold]).validate().is_err());

    let mut directory = skill("safe-skill", "Safe Skill");
    for path in ["scripts", "scripts-old", "scripts/extra.sh"] {
        directory.files.push(PortableSkillFile {
            path: path.to_string(),
            bytes: b"content".to_vec(),
            executable: false,
        });
    }
    assert!(bundle(vec![directory]).validate().is_err());
}

#[test]
fn inspection_preserves_all_agent_skills_frontmatter() {
    let mut candidate = skill("safe-skill", "Safe Skill");
    candidate.files[0].bytes = skill_md(
        "safe-skill",
        "Safe Skill",
        "license: Apache-2.0\ncompatibility: Requires network access\nmetadata:\n  author: Buzz\nallowed-tools: Read\n",
    );
    let inspection = bundle(vec![candidate]).inspection().unwrap();
    let skill = &inspection.skills[0];
    assert_eq!(skill.license.as_deref(), Some("Apache-2.0"));
    assert_eq!(
        skill.compatibility.as_deref(),
        Some("Requires network access")
    );
    assert_eq!(
        skill.metadata.get("author").map(String::as_str),
        Some("Buzz")
    );
    assert_eq!(skill.requested_allowed_tools.as_deref(), Some("Read"));
}

#[test]
fn single_skill_inspection_validates_the_directory_identity() {
    let bytes = skill_md(
        "safe-skill",
        "Safe Skill",
        "license: Apache-2.0\nallowed-tools: Read\n",
    );
    let metadata = inspect_skill_md("safe-skill", &bytes).unwrap();
    assert_eq!(metadata.name(), "safe-skill");
    assert_eq!(metadata.description(), "Safe Skill");
    assert_eq!(metadata.license(), Some("Apache-2.0"));
    assert_eq!(metadata.compatibility(), None);
    assert!(metadata.metadata().is_empty());
    assert_eq!(metadata.requested_allowed_tools(), Some("Read"));
    assert!(inspect_skill_md("different-skill", &bytes).is_err());
}

#[test]
fn single_skill_inspection_is_bounded_before_parsing() {
    let oversized = vec![b'x'; MAX_PORTABLE_SKILL_FILE_BYTES + 1];
    assert!(inspect_skill_md("safe-skill", &oversized)
        .unwrap_err()
        .contains("exceeds"));
}

#[test]
fn accepts_spec_valid_empty_and_whitespace_optional_metadata() {
    let candidate = PortableSkill {
        name: "safe-skill".to_string(),
        description: "Safe Skill".to_string(),
        files: vec![PortableSkillFile {
            path: "SKILL.md".to_string(),
            bytes: b"---\nname: safe-skill\ndescription: Safe Skill\nlicense: '  Custom license  '\nmetadata:\n  '': ''\nallowed-tools: ''\n---\nBody"
                .to_vec(),
            executable: false,
        }],
    };
    let inspection = bundle(vec![candidate]).inspection().unwrap();
    assert_eq!(
        inspection.skills[0].license.as_deref(),
        Some("  Custom license  ")
    );
    assert_eq!(
        inspection.skills[0].metadata.get("").map(String::as_str),
        Some("")
    );
    assert_eq!(
        inspection.skills[0].requested_allowed_tools.as_deref(),
        Some("")
    );
}

#[test]
fn rejects_empty_compatibility_when_present() {
    let bytes = b"---\nname: safe-skill\ndescription: Safe Skill\ncompatibility: ''\n---\nBody";
    assert!(inspect_skill_md("safe-skill", bytes)
        .unwrap_err()
        .contains("1 to 500"));
}

#[test]
fn accepts_deep_paths_within_the_portable_length_limit() {
    let mut candidate = skill("safe-skill", "Safe Skill");
    let path = format!("{}/file.txt", vec!["nested"; 20].join("/"));
    candidate.files.push(PortableSkillFile {
        path,
        bytes: b"content".to_vec(),
        executable: false,
    });
    bundle(vec![candidate]).validate().unwrap();
}

#[test]
fn rejects_unknown_or_malformed_agent_skills_frontmatter() {
    let mut unknown = skill("safe-skill", "Safe Skill");
    unknown.files[0].bytes =
        b"---\nname: safe-skill\ndescription: Safe Skill\nruntime-policy: unrestricted\n---\nBody"
            .to_vec();
    assert!(bundle(vec![unknown]).validate().is_err());

    let mut non_string_metadata = skill("safe-skill", "Safe Skill");
    non_string_metadata.files[0].bytes =
        b"---\nname: safe-skill\ndescription: Safe Skill\nmetadata:\n  version: 1\n---\nBody"
            .to_vec();
    assert!(bundle(vec![non_string_metadata]).validate().is_err());

    let mut duplicate = skill("safe-skill", "Safe Skill");
    duplicate.files[0].bytes =
        b"---\nname: safe-skill\nname: other\ndescription: Safe Skill\n---\nBody".to_vec();
    assert!(bundle(vec![duplicate]).validate().is_err());
}

#[test]
fn duplicate_skills_and_unknown_bundle_versions_fail_closed() {
    let duplicate = skill("safe-skill", "Safe Skill");
    assert!(bundle(vec![duplicate.clone(), duplicate])
        .validate()
        .is_err());

    let mut unknown = bundle(vec![skill("safe-skill", "Safe Skill")]);
    unknown.schema_version += 1;
    assert!(unknown.validate().is_err());
}

#[test]
fn bundle_size_is_bounded_before_digest_or_inspection() {
    let mut candidate = skill("safe-skill", "Safe Skill");
    for index in 0..4 {
        candidate.files.push(PortableSkillFile {
            path: format!("assets/large-{index}.bin"),
            bytes: vec![index as u8; MAX_PORTABLE_SKILL_FILE_BYTES],
            executable: false,
        });
    }
    let oversized = bundle(vec![candidate]);
    assert!(oversized.validate().unwrap_err().contains("in total"));
    assert!(oversized.canonical_digest().is_err());
    assert!(oversized.inspection().is_err());
}

#[test]
fn secret_scan_is_advisory_and_never_claims_binary_safety() {
    let mut risky = skill("database-access", "Database access");
    risky.files.push(PortableSkillFile {
        path: "references/connection.txt".to_string(),
        bytes: b"DATABASE_URL=postgres://user:real-password@host/database".to_vec(),
        executable: false,
    });
    let candidate = bundle(vec![risky]);
    let warnings = candidate.review_warnings().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].message.contains("may contain"));
    assert!(candidate.inspection().unwrap().skills[0]
        .files
        .iter()
        .any(|file| file.path == "assets/status.png" && !file.is_utf8));
}

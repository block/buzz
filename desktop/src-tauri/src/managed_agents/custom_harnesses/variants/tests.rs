//! Regression tests for profile-variant expansion.
//!
//! Every test below drives `load_custom_harnesses`, the seam the whole family
//! flow runs through, so deleting the expansion call in the loader, the
//! `variants` validation, or the `label_from` read fails these tests.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::super::tests::make_def;
use super::super::{
    load_custom_harnesses, save_custom_harness_to_dir, validate_harness_definition,
    HarnessDefinition,
};
use super::{HarnessVariantLabel, HarnessVariants, MAX_HARNESS_VARIANTS};

fn variant_template(parent_dir: &str) -> HarnessDefinition {
    HarnessDefinition {
        // NOT "hermes": `presets.rs` still ships a preset with that id, so
        // `check_id_collision` rejects it and the loader would drop the
        // template entry before these tests ever see it. The shipped
        // declaration uses `hermes-profiles` for the same reason.
        id: "hermes-profiles".to_string(),
        label: "Hermes Agent".to_string(),
        command: "hermes-acp".to_string(),
        variants: Some(HarnessVariants {
            dir: parent_dir.to_string(),
            marker: "profile.yaml".to_string(),
            max: None,
            // Keep the generated ids short so the assertions below stay
            // readable; the default (`{id}-{slug}`) is covered by the
            // template's own id.
            id_template: Some("hermes-{slug}".to_string()),
            label_template: None,
            label_from: None,
            env: BTreeMap::from([("HERMES_HOME".to_string(), "{dir}".to_string())]),
            args: Vec::new(),
        }),
        ..Default::default()
    }
}

/// Write a profile directory (carrying the `profile.yaml` marker) under
/// `parent` and return its path.
fn write_profile(parent: &Path, name: &str, profile_yaml: &str) -> PathBuf {
    let dir = parent.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("profile.yaml"), profile_yaml).unwrap();
    dir
}

/// Persist `def` through the real save path and read it back through the
/// real loader — the same two calls the app uses.
fn save_and_load(harness_dir: &Path, def: &HarnessDefinition) -> Vec<HarnessDefinition> {
    save_custom_harness_to_dir(harness_dir, def, None).unwrap();
    load_custom_harnesses(harness_dir)
}

#[test]
fn definition_without_variants_loads_as_a_single_entry() {
    let harness_dir = tempfile::tempdir().unwrap();

    let loaded = save_and_load(harness_dir.path(), &make_def("plain", "Plain"));

    assert_eq!(loaded.len(), 1, "no variants block means no expansion");
    assert_eq!(loaded[0].id, "plain");
    assert!(!loaded[0].generated);
}

#[test]
fn variants_expand_one_entry_per_profile_directory() {
    let harness_dir = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let alpha = write_profile(parent.path(), "alpha", "version: 1\n");
    write_profile(parent.path(), "beta", "version: 1\n");

    let def = variant_template(&parent.path().to_string_lossy());
    let loaded = save_and_load(harness_dir.path(), &def);

    assert_eq!(loaded.len(), 3, "template plus two profiles: {loaded:#?}");

    let template = loaded.iter().find(|d| d.id == "hermes-profiles").unwrap();
    assert!(
        !template.generated,
        "the file-backed template entry is not generated"
    );
    assert!(template.variants.is_some(), "the template keeps its block");
    assert!(
        template.env.is_empty(),
        "the template carries none of the variant env"
    );

    let alpha_entry = loaded.iter().find(|d| d.id == "hermes-alpha").unwrap();
    assert!(alpha_entry.generated);
    assert_eq!(
        alpha_entry.generated_from.as_deref(),
        Some("hermes-profiles"),
        "a variant names the definition that produced it so the UI can point at that file"
    );
    assert_eq!(alpha_entry.label, "Hermes Agent (alpha)");
    assert_eq!(alpha_entry.command, "hermes-acp");
    assert!(
        alpha_entry.variants.is_none(),
        "a materialized variant is not another template"
    );
    let alpha_home = alpha.to_string_lossy().to_string();
    assert_eq!(
        alpha_entry.env.get("HERMES_HOME").map(String::as_str),
        Some(alpha_home.as_str()),
        "each variant points its env at its own profile directory"
    );
    assert_eq!(loaded.iter().filter(|d| d.generated).count(), 2);
}

#[test]
fn variants_read_label_from_profile_metadata() {
    let harness_dir = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    write_profile(
        parent.path(),
        "generalist",
        "version: 1\nui_meta:\n  hermes-bots:\n    title: Generalist\n",
    );

    let mut def = variant_template(&parent.path().to_string_lossy());
    {
        let variants = def.variants.as_mut().unwrap();
        variants.label_template = Some("Hermes Agent [{meta}] ({name})".to_string());
        variants.label_from = Some(HarnessVariantLabel {
            file: "profile.yaml".to_string(),
            key: "ui_meta.hermes-bots.title".to_string(),
        });
    }
    let loaded = save_and_load(harness_dir.path(), &def);

    let entry = loaded.iter().find(|d| d.generated).unwrap();
    assert_eq!(entry.id, "hermes-generalist");
    assert_eq!(
        entry.label, "Hermes Agent [Generalist] (generalist)",
        "the persona title fills the bracket; the directory name stays the suffix"
    );
}

#[test]
fn variants_default_label_includes_metadata_when_no_template_is_declared() {
    let harness_dir = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    write_profile(
        parent.path(),
        "generalist",
        "ui_meta:\n  hermes-bots:\n    title: Generalist\n",
    );

    let mut def = variant_template(&parent.path().to_string_lossy());
    def.variants.as_mut().unwrap().label_from = Some(HarnessVariantLabel {
        file: "profile.yaml".to_string(),
        key: "ui_meta.hermes-bots.title".to_string(),
    });
    let loaded = save_and_load(harness_dir.path(), &def);

    let entry = loaded.iter().find(|d| d.generated).unwrap();
    assert_eq!(entry.label, "Hermes Agent [Generalist] (generalist)");
}

#[test]
fn variants_fall_back_to_a_plain_label_when_metadata_is_unreadable() {
    let harness_dir = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    // Marker present, `ui_meta` absent: the profile still appears.
    write_profile(parent.path(), "generalist", "version: 1\n");

    let mut def = variant_template(&parent.path().to_string_lossy());
    {
        let variants = def.variants.as_mut().unwrap();
        variants.label_template = Some("Hermes Agent [{meta}] ({name})".to_string());
        variants.label_from = Some(HarnessVariantLabel {
            file: "profile.yaml".to_string(),
            key: "ui_meta.hermes-bots.title".to_string(),
        });
    }
    let loaded = save_and_load(harness_dir.path(), &def);

    let entry = loaded.iter().find(|d| d.generated).unwrap();
    assert_eq!(
        entry.label, "Hermes Agent (generalist)",
        "an unreadable value must not render as an empty bracket"
    );
}

#[test]
fn variants_skip_directories_without_the_marker() {
    let harness_dir = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    write_profile(parent.path(), "alpha", "version: 1\n");
    fs::create_dir_all(parent.path().join("not-a-profile")).unwrap();
    fs::write(parent.path().join("loose-file.json"), "{}").unwrap();

    let def = variant_template(&parent.path().to_string_lossy());
    let loaded = save_and_load(harness_dir.path(), &def);

    let mut ids: Vec<&str> = loaded.iter().map(|d| d.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["hermes-alpha", "hermes-profiles"]);
}

#[test]
fn variants_are_capped_at_the_documented_maximum() {
    let harness_dir = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    for index in 0..(MAX_HARNESS_VARIANTS + 6) {
        write_profile(parent.path(), &format!("p{index:03}"), "version: 1\n");
    }

    let def = variant_template(&parent.path().to_string_lossy());
    let loaded = save_and_load(harness_dir.path(), &def);

    assert_eq!(
        loaded.iter().filter(|d| d.generated).count(),
        MAX_HARNESS_VARIANTS,
        "a template pointed at a huge directory must not flood the catalog"
    );
    assert_eq!(loaded.len(), MAX_HARNESS_VARIANTS + 1);
}

#[test]
fn variants_honour_a_declared_max() {
    let harness_dir = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    for name in ["alpha", "beta", "gamma"] {
        write_profile(parent.path(), name, "version: 1\n");
    }

    let mut def = variant_template(&parent.path().to_string_lossy());
    def.variants.as_mut().unwrap().max = Some(2);
    let loaded = save_and_load(harness_dir.path(), &def);

    let mut ids: Vec<&str> = loaded.iter().map(|d| d.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec!["hermes-alpha", "hermes-beta", "hermes-profiles"],
        "the cap applies to generated entries only; the template always survives"
    );
}

#[test]
fn variants_validation_rejects_label_files_outside_the_profile_dir() {
    let mut def = variant_template("~/.hermes/profiles");
    fn with_file(def: &mut HarnessDefinition, file: &str) {
        def.variants.as_mut().unwrap().label_from = Some(HarnessVariantLabel {
            file: file.to_string(),
            key: "ui_meta.hermes-bots.title".to_string(),
        });
    }

    for escaping in [
        "../secrets.yaml",
        "nested/../../up.yaml",
        "/etc/passwd",
        "\\Windows\\win.ini",
        "C:\\secrets.yaml",
        "C:secrets.yaml",
    ] {
        with_file(&mut def, escaping);
        let err = validate_harness_definition(&def)
            .expect_err("a label file outside the profile directory must be rejected");
        assert!(
            format!("{err:?}").contains("labelFrom.file"),
            "the error must name the offending field, got: {err:?}"
        );
    }

    with_file(&mut def, "nested/meta.yaml");
    assert!(
        validate_harness_definition(&def).is_ok(),
        "a nested relative path inside the profile directory stays legal"
    );
}

#[test]
fn variants_validation_rejects_empty_label_file_or_key() {
    let mut def = variant_template("~/.hermes/profiles");
    def.variants.as_mut().unwrap().label_from = Some(HarnessVariantLabel {
        file: "   ".to_string(),
        key: "ui_meta.hermes-bots.title".to_string(),
    });
    assert!(validate_harness_definition(&def).is_err());

    def.variants.as_mut().unwrap().label_from = Some(HarnessVariantLabel {
        file: "profile.yaml".to_string(),
        key: "".to_string(),
    });
    assert!(validate_harness_definition(&def).is_err());
}

#[test]
fn variants_validation_rejects_env_that_the_spawn_path_would_reject() {
    let mut def = variant_template("~/.hermes/profiles");
    def.variants
        .as_mut()
        .unwrap()
        .env
        .insert("BUZZ_AUTH_TAG".to_string(), "forged".to_string());

    let err = validate_harness_definition(&def)
        .expect_err("a reserved key inside variants.env must be rejected at load");
    assert!(
        format!("{err:?}").contains("variants.env"),
        "the error must name the offending block, got: {err:?}"
    );
}

#[test]
fn variants_with_an_unreadable_dir_still_load_the_template() {
    let harness_dir = tempfile::tempdir().unwrap();
    let missing = harness_dir.path().join("no-such-parent");

    let def = variant_template(&missing.to_string_lossy());
    let loaded = save_and_load(harness_dir.path(), &def);

    assert_eq!(
        loaded.len(),
        1,
        "an unreadable variants dir degrades to the bare template, never to nothing"
    );
    assert_eq!(loaded[0].id, "hermes-profiles");
}

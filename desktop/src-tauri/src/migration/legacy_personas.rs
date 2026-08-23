use std::path::Path;

use tauri::Manager;

const RENAMES: &[(&str, &str)] = &[
    ("builtin:fizz", "builtin:diego"),
    ("builtin:honey", "builtin:murietta"),
    ("builtin:bumble", "builtin:montero"),
    ("Fizz", "Diego"),
    ("Honey", "Murietta"),
    // Preserve upstream's profile-safe staged rename: Bumble becomes the
    // intermediate Pollen spelling here, then `migration::pollen` advances
    // Pollen to Montero and queues relay profile reconciliation.
    ("Bumble", "Pollen"),
];

fn rewrite_strings(value: &mut serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => {
            let rewritten = RENAMES
                .iter()
                .fold(text.clone(), |text, (old, new)| text.replace(old, new));
            if rewritten == *text {
                false
            } else {
                *text = rewritten;
                true
            }
        }
        serde_json::Value::Array(values) => {
            let mut changed = false;
            for value in values {
                changed = rewrite_strings(value) || changed;
            }
            changed
        }
        serde_json::Value::Object(values) => {
            let mut changed = false;
            for value in values.values_mut() {
                changed = rewrite_strings(value) || changed;
            }
            changed
        }
        _ => false,
    }
}

fn rename_in_file(path: &Path) {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&contents) else {
        eprintln!(
            "zorro-desktop: built-in-persona-rename: invalid JSON in {}",
            path.display()
        );
        return;
    };
    if !rewrite_strings(&mut value) {
        return;
    }
    match serde_json::to_vec_pretty(&value) {
        Ok(json) => {
            if let Err(error) = crate::managed_agents::atomic_write_json_restricted(path, &json) {
                eprintln!(
                    "zorro-desktop: built-in-persona-rename: could not update {}: {error}",
                    path.display()
                );
            }
        }
        Err(error) => eprintln!(
            "zorro-desktop: built-in-persona-rename: could not serialize {}: {error}",
            path.display()
        ),
    }
}

pub(super) fn rename_legacy_built_in_personas(app: &tauri::AppHandle) {
    let Ok(dir) = app.path().app_data_dir() else {
        return;
    };
    for relative_path in [
        "agents/managed-agents.json",
        "agents/personas.json",
        "agents/teams.json",
    ] {
        let path = dir.join(relative_path);
        if path.exists() {
            rename_in_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_updates_ids_and_stages_bumble_for_profile_reconciliation() {
        let mut value = serde_json::json!([{
            "id": "builtin:fizz",
            "display_name": "Fizz",
            "system_prompt": "You are Fizz.",
            "research_prompt": "You are Bumble.",
            "persona_ids": ["builtin:honey", "builtin:bumble"]
        }]);

        assert!(rewrite_strings(&mut value));
        assert_eq!(value[0]["id"], "builtin:diego");
        assert_eq!(value[0]["display_name"], "Diego");
        assert_eq!(value[0]["system_prompt"], "You are Diego.");
        assert_eq!(value[0]["research_prompt"], "You are Pollen.");
        assert_eq!(
            value[0]["persona_ids"],
            serde_json::json!(["builtin:murietta", "builtin:montero"])
        );
        assert!(!rewrite_strings(&mut value));
    }

    #[test]
    fn rename_prepares_intermediate_pollen_for_profile_safe_migration() {
        let mut value = serde_json::json!([{
            "id": "builtin:bumble",
            "display_name": "Pollen",
            "system_prompt": "You are Pollen."
        }]);

        assert!(rewrite_strings(&mut value));
        assert_eq!(value[0]["id"], "builtin:montero");
        assert_eq!(value[0]["display_name"], "Pollen");
        assert_eq!(value[0]["system_prompt"], "You are Pollen.");
        assert!(!rewrite_strings(&mut value));
    }
}

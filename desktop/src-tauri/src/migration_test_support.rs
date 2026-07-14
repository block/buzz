//! Shared helpers for the migration test modules.

use std::path::Path;

pub(crate) fn write_agents_json(dir: &Path, records: &serde_json::Value) {
    std::fs::create_dir_all(dir.join("agents")).unwrap();
    std::fs::write(
        dir.join("agents/managed-agents.json"),
        serde_json::to_vec_pretty(records).unwrap(),
    )
    .unwrap();
}

pub(crate) fn read_agents_json(dir: &Path) -> Vec<serde_json::Value> {
    let content = std::fs::read_to_string(dir.join("agents/managed-agents.json")).unwrap();
    serde_json::from_str(&content).unwrap()
}

pub(crate) fn write_personas_json(dir: &Path, records: &serde_json::Value) {
    std::fs::create_dir_all(dir.join("agents")).unwrap();
    std::fs::write(
        dir.join("agents/personas.json"),
        serde_json::to_vec_pretty(records).unwrap(),
    )
    .unwrap();
}

pub(crate) fn read_personas_json(dir: &Path) -> Vec<serde_json::Value> {
    let content = std::fs::read_to_string(dir.join("agents/personas.json")).unwrap();
    serde_json::from_str(&content).unwrap()
}

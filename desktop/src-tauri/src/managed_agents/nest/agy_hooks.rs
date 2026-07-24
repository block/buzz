use std::fs;
use std::io;
use std::path::Path;

/// Antigravity hooks that translate documented tool lifecycle events into
/// Buzz ACP observer updates.
const AGY_HOOKS_JSON: &str = include_str!("../nest_agy_hooks.json");
pub(super) const AGY_HOOKS_KEY: &str = "buzz-antigravity-observer";

/// Add Buzz's Antigravity hook observer without replacing user-defined hooks.
///
/// A malformed existing hooks file is preserved unchanged. Antigravity will
/// report that configuration problem itself, and Buzz must not destroy a
/// user's customizations while trying to repair it.
pub(super) fn ensure_agy_hooks(root: &Path) -> Result<(), String> {
    let agents_dir = root.join(".agents");
    fs::create_dir_all(&agents_dir).map_err(|e| format!("create {}: {e}", agents_dir.display()))?;
    let hooks_path = agents_dir.join("hooks.json");
    let template: serde_json::Value = serde_json::from_str(AGY_HOOKS_JSON)
        .map_err(|e| format!("parse bundled Antigravity hooks: {e}"))?;
    let Some(template_hook) = template.get(AGY_HOOKS_KEY).cloned() else {
        return Err("bundled Antigravity hooks are missing their managed key".to_string());
    };

    let mut hooks = match fs::read_to_string(&hooks_path) {
        Ok(existing) => match serde_json::from_str::<serde_json::Value>(&existing) {
            Ok(serde_json::Value::Object(hooks)) => hooks,
            Ok(_) | Err(_) => return Ok(()),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => serde_json::Map::new(),
        Err(error) => return Err(format!("read {}: {error}", hooks_path.display())),
    };
    if hooks.contains_key(AGY_HOOKS_KEY) {
        return Ok(());
    }
    hooks.insert(AGY_HOOKS_KEY.to_string(), template_hook);

    let encoded = serde_json::to_vec_pretty(&serde_json::Value::Object(hooks))
        .map_err(|e| format!("serialize Antigravity hooks: {e}"))?;
    let mut tmp = tempfile::NamedTempFile::new_in(&agents_dir)
        .map_err(|e| format!("tempfile in {}: {e}", agents_dir.display()))?;
    {
        use std::io::Write;
        tmp.write_all(&encoded)
            .map_err(|e| format!("write Antigravity hooks tempfile: {e}"))?;
        tmp.write_all(b"\n")
            .map_err(|e| format!("finish Antigravity hooks tempfile: {e}"))?;
    }
    tmp.persist(&hooks_path)
        .map_err(|e| format!("persist {}: {e}", hooks_path.display()))?;
    Ok(())
}

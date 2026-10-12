//! Interactive permission prompts from managed agents (opt-in per agent with the env var
//! `BUZZ_ACP_PERMISSION_MODE=ask`). buzz-acp writes `<id>.req.json` into the agent's
//! private prompt directory and waits for `<id>.ans.json`; these commands list and answer
//! them, locally or from the web over Direct (see bridge/directCmd.ts, which redacts).

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use tauri::AppHandle;

use crate::managed_agents::managed_agents_base_dir;

pub const PERMISSION_DIR_ENV: &str = "BUZZ_ACP_PERMISSION_DIR";

fn valid_pubkey(pk: &str) -> Result<String, String> {
    let pk = pk.trim().to_ascii_lowercase();
    if pk.len() == 64 && pk.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(pk)
    } else {
        Err("invalid agent pubkey".into())
    }
}

fn valid_request_id(id: &str) -> Result<&str, String> {
    if !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
        Ok(id)
    } else {
        Err("invalid permission request id".into())
    }
}

pub fn prompt_dir(base: &Path, pubkey: &str) -> PathBuf {
    base.join("permission-prompts").join(pubkey)
}

/// Create the agent's prompt directory (owner-only) and return it for the spawn env.
pub fn ensure_prompt_dir<R: tauri::Runtime>(
    app: &AppHandle<R>,
    pubkey: &str,
) -> Result<PathBuf, String> {
    let dir = prompt_dir(&managed_agents_base_dir(app)?, &valid_pubkey(pubkey)?);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionPrompt {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub detail: String,
    pub options: Vec<PermissionOption>,
    pub created_at: u64,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: String,
}

fn parse_prompt(raw: &str) -> Option<PermissionPrompt> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let id = s("id");
    valid_request_id(&id).ok()?;
    Some(PermissionPrompt {
        options: v
            .get("options")?
            .as_array()?
            .iter()
            .filter_map(|o| {
                Some(PermissionOption {
                    option_id: o.get("optionId")?.as_str()?.to_string(),
                    name: o
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    kind: o
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                })
            })
            .take(10)
            .collect(),
        id,
        title: s("title"),
        kind: s("kind"),
        detail: s("detail"),
        created_at: v.get("createdAt").and_then(Value::as_u64).unwrap_or(0),
    })
}

pub fn list_in(dir: &Path) -> Vec<PermissionPrompt> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PermissionPrompt> = rd
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".req.json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|raw| parse_prompt(&raw))
        .filter(|p| !dir.join(format!("{}.ans.json", p.id)).exists())
        .take(50)
        .collect();
    out.sort_by_key(|p| p.created_at);
    out
}

pub fn answer_in(dir: &Path, request_id: &str, option_id: &str) -> Result<(), String> {
    let id = valid_request_id(request_id)?;
    let req = dir.join(format!("{id}.req.json"));
    let raw = std::fs::read_to_string(&req)
        .map_err(|_| "That permission request is no longer pending.".to_string())?;
    let prompt = parse_prompt(&raw).ok_or("unreadable permission request")?;
    if !prompt.options.iter().any(|o| o.option_id == option_id) {
        return Err("That option isn't offered for this request.".into());
    }
    let ans = dir.join(format!("{id}.ans.json"));
    let tmp = dir.join(format!("{id}.ans.tmp"));
    std::fs::write(
        &tmp,
        serde_json::json!({ "optionId": option_id }).to_string(),
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(tmp, ans).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_permission_prompts(
    pubkey: String,
    app: AppHandle,
) -> Result<Vec<PermissionPrompt>, String> {
    let dir = prompt_dir(&managed_agents_base_dir(&app)?, &valid_pubkey(&pubkey)?);
    Ok(list_in(&dir))
}

#[tauri::command]
pub fn respond_permission_prompt(
    pubkey: String,
    request_id: String,
    option_id: String,
    app: AppHandle,
) -> Result<(), String> {
    let dir = prompt_dir(&managed_agents_base_dir(&app)?, &valid_pubkey(&pubkey)?);
    answer_in(&dir, &request_id, &option_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(dir: &Path, id: &str) {
        let body = serde_json::json!({
            "id": id, "title": "Run ls", "kind": "execute", "detail": "ls", "createdAt": 5,
            "options": [{ "optionId": "a1", "name": "Allow", "kind": "allow_once" }, { "optionId": "r1", "name": "Reject", "kind": "reject_once" }]
        });
        std::fs::write(dir.join(format!("{id}.req.json")), body.to_string()).unwrap();
    }

    #[test]
    fn lists_and_answers_pending_prompts() {
        let d = tempfile::tempdir().unwrap();
        req(d.path(), "12-34");
        let l = list_in(d.path());
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].options.len(), 2);
        assert!(answer_in(d.path(), "12-34", "zz").is_err());
        assert!(answer_in(d.path(), "../x", "a1").is_err());
        answer_in(d.path(), "12-34", "r1").unwrap();
        let ans = std::fs::read_to_string(d.path().join("12-34.ans.json")).unwrap();
        assert!(ans.contains("r1"));
        assert!(list_in(d.path()).is_empty());
        assert!(answer_in(d.path(), "99", "a1").is_err());
    }

    #[test]
    fn rejects_bad_pubkeys() {
        assert!(valid_pubkey("x").is_err());
        assert!(valid_pubkey(&"A".repeat(64)).is_ok());
    }
}

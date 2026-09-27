//! Drive action helpers (JS payloads executed in the granted webview).

use serde::{Deserialize, Serialize};

/// Ghost-cursor / page actions. Field is `kind` (not `type`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveAction {
    pub kind: String,
    /// Caller- or host-assigned id; echoed in the page result and `drive` event.
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub selector: Option<String>,
    /// Snapshot interactive ref (e.g. "e0") from the latest browser_snapshot.
    #[serde(default, rename = "ref")]
    pub ref_id: Option<String>,
    #[serde(default)]
    pub dx: Option<f64>,
    #[serde(default)]
    pub dy: Option<f64>,
    /// For `kind: "key"` — Enter, Tab, Escape, Backspace, ArrowLeft/Right/Up/Down.
    #[serde(default)]
    pub key: Option<String>,
    /// For `kind: "waitFor"` — urlContains | selector | text.
    #[serde(default)]
    pub url_contains: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// For `kind: "fill"` — clear existing value before typing (default true).
    #[serde(default)]
    pub clear: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveHit {
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveActionResult {
    pub id: String,
    pub ok: bool,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit: Option<DriveHit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Legacy/compat short message (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub const DRIVE_RESULT_COOKIE: &str = "__buzz_ba_drive_result";

/// Validate action shape before queue/eval. Returns normalized kind.
pub fn validate_action(action: &DriveAction) -> Result<String, String> {
    let kind = action.kind.trim().to_ascii_lowercase();
    match kind.as_str() {
        "click" | "hover" => {
            let has_xy = action.x.is_some() && action.y.is_some();
            let has_sel = action
                .selector
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some();
            let has_ref = action
                .ref_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some();
            if !has_xy && !has_sel && !has_ref {
                return Err(format!("{kind} requires x,y or selector or ref"));
            }
        }
        "type" => {
            if action.text.is_none() {
                return Err("type requires text".into());
            }
        }
        "fill" => {
            if action.text.is_none() {
                return Err("fill requires text".into());
            }
            let has_xy = action.x.is_some() && action.y.is_some();
            let has_sel = action
                .selector
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some();
            let has_ref = action
                .ref_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some();
            if !has_xy && !has_sel && !has_ref {
                return Err("fill requires x,y or selector or ref".into());
            }
        }
        "scroll" => {}
        "navigate" => {
            if action.url.as_deref().map(str::trim).unwrap_or("").is_empty() {
                return Err("navigate requires url".into());
            }
        }
        "key" => {
            let key = action.key.as_deref().map(str::trim).unwrap_or("");
            if key.is_empty() {
                return Err("key requires key".into());
            }
            if !SUPPORTED_KEYS
                .iter()
                .any(|k| k.eq_ignore_ascii_case(key))
            {
                return Err(format!(
                    "unsupported key {key:?}; supported: {}",
                    SUPPORTED_KEYS.join(", ")
                ));
            }
        }
        "waitfor" => {
            let has = action
                .url_contains
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_some()
                || action
                    .selector
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .is_some()
                || action
                    .text
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .is_some();
            if !has {
                return Err("waitFor requires urlContains, selector, or text".into());
            }
        }
        "" => return Err("action.kind is required".into()),
        other => return Err(format!("unknown drive action kind: {other}")),
    }
    Ok(kind)
}

const SUPPORTED_KEYS: &[&str] = &[
    "Enter",
    "Tab",
    "Escape",
    "Backspace",
    "ArrowLeft",
    "ArrowRight",
    "ArrowUp",
    "ArrowDown",
];

pub fn ensure_action_id(action: &mut DriveAction) -> String {
    if let Some(id) = action.id.as_ref().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    {
        action.id = Some(id.clone());
        return id;
    }
    let id = format!(
        "d{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    action.id = Some(id.clone());
    id
}

/// Build page JS. Caller must drain `DRIVE_RESULT_COOKIE` (or page queue) after eval.
/// Returns the JS string; page result is written to the cookie + `__buzzDriveLastResult`.
pub fn action_js(action: &DriveAction) -> Result<String, String> {
    let kind = validate_action(action)?;
    let id = action
        .id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown");
    let id_js = serde_json::to_string(id).map_err(|e| e.to_string())?;

    let call = match kind.as_str() {
        "click" | "hover" => {
            let x_js = action
                .x
                .map(|v| v.to_string())
                .unwrap_or_else(|| "null".into());
            let y_js = action
                .y
                .map(|v| v.to_string())
                .unwrap_or_else(|| "null".into());
            let selector_js = match action
                .selector
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
                None => "null".into(),
            };
            let ref_js = match action
                .ref_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
                None => "null".into(),
            };
            let fn_name = if kind == "click" { "clickAt" } else { "hoverAt" };
            format!("a.{fn_name}({x_js},{y_js},{id_js},{selector_js},{ref_js})")
        }
        "type" => {
            let text = action.text.as_deref().unwrap_or("");
            let text_js = serde_json::to_string(text).map_err(|e| e.to_string())?;
            let selector_js = match &action.selector {
                Some(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
                None => "null".into(),
            };
            format!("a.typeText({text_js},{selector_js},{id_js})")
        }
        "fill" => {
            let text = action.text.as_deref().unwrap_or("");
            let text_js = serde_json::to_string(text).map_err(|e| e.to_string())?;
            let selector_js = match action
                .selector
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
                None => "null".into(),
            };
            let ref_js = match action
                .ref_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
                None => "null".into(),
            };
            let x_js = action
                .x
                .map(|v| v.to_string())
                .unwrap_or_else(|| "null".into());
            let y_js = action
                .y
                .map(|v| v.to_string())
                .unwrap_or_else(|| "null".into());
            let clear = action.clear.unwrap_or(true);
            format!(
                "a.fillField({text_js},{selector_js},{ref_js},{x_js},{y_js},{id_js},{clear})"
            )
        }
        "scroll" => {
            let dx = action.dx.unwrap_or(0.0);
            let dy = action.dy.unwrap_or(0.0);
            format!("a.scrollBy({dx},{dy},{id_js})")
        }
        "key" => {
            let key = action.key.as_deref().unwrap_or("");
            let key_js = serde_json::to_string(key).map_err(|e| e.to_string())?;
            format!("a.pressKey({key_js},{id_js})")
        }
        "waitfor" => {
            let url_js = match &action.url_contains {
                Some(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
                None => "null".into(),
            };
            let sel_js = match &action.selector {
                Some(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
                None => "null".into(),
            };
            let text_js = match &action.text {
                Some(s) => serde_json::to_string(s).map_err(|e| e.to_string())?,
                None => "null".into(),
            };
            let timeout = action.timeout_ms.unwrap_or(10_000).min(60_000);
            // Synchronous check only; host polls by re-eval until ok or timeout.
            format!("a.waitForCheck({url_js},{sel_js},{text_js},{id_js},{timeout})")
        }
        "navigate" => return Err("navigate is handled by the host".into()),
        other => return Err(format!("unknown drive action: {other}")),
    };

    Ok(format!(
        r#"(function(){{
  try {{ document.cookie='__buzz_ba_drive_result=; path=/; SameSite=Lax'; }} catch(e) {{}}
  (async function(){{
    var a=window.__buzzBrowserAgent;
    var r;
    if(!a){{
      r=JSON.stringify({{id:{id_js},ok:false,kind:{kind_js},error:'no agent instrumentation'}});
    }} else {{
      try {{
        var out={call};
        r = (out && typeof out.then === 'function') ? await out : out;
      }} catch(e) {{
        r=JSON.stringify({{id:{id_js},ok:false,kind:{kind_js},error:String(e)}});
      }}
    }}
    if(typeof r!=='string') r=JSON.stringify(r||{{id:{id_js},ok:false,kind:{kind_js},error:'empty result'}});
    try {{
      document.cookie='__buzz_ba_drive_result='+encodeURIComponent(r)+'; path=/; SameSite=Lax';
    }} catch(e) {{}}
    try {{ window.__buzzDriveLastResult=r; }} catch(e) {{}}
  }})();
  return 'pending';
}})();"#,
        kind_js = serde_json::to_string(&kind).unwrap_or_else(|_| "\"unknown\"".into()),
    ))
}

pub fn navigate_result(id: &str, url: &str) -> DriveActionResult {
    DriveActionResult {
        id: id.to_string(),
        ok: true,
        kind: "navigate".into(),
        hit: None,
        url: Some(url.to_string()),
        error: None,
        message: Some("navigated".into()),
    }
}

pub fn error_result(id: &str, kind: &str, error: impl Into<String>) -> DriveActionResult {
    DriveActionResult {
        id: id.to_string(),
        ok: false,
        kind: kind.to_string(),
        hit: None,
        url: None,
        error: Some(error.into()),
        message: None,
    }
}

pub fn parse_page_result(raw: &str) -> Option<DriveActionResult> {
    serde_json::from_str(raw).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(kind: &str) -> DriveAction {
        DriveAction {
            kind: kind.into(),
            id: Some("t1".into()),
            url: None,
            x: None,
            y: None,
            text: None,
            selector: None,
            ref_id: None,
            dx: None,
            dy: None,
            key: None,
            url_contains: None,
            timeout_ms: None,
            clear: None,
        }
    }

    #[test]
    fn click_js_includes_coordinates_and_returns_result() {
        let mut a = base("click");
        a.x = Some(10.0);
        a.y = Some(20.0);
        let js = action_js(&a).unwrap();
        assert!(js.contains("clickAt(10,20,\"t1\",null,null)"), "{js}");
        assert!(js.contains("__buzz_ba_drive_result"));
        assert!(js.contains("await out"));
    }

    #[test]
    fn key_js_presses_enter() {
        let mut a = base("key");
        a.key = Some("Enter".into());
        let js = action_js(&a).unwrap();
        assert!(js.contains("pressKey(\"Enter\",\"t1\")"));
    }

    #[test]
    fn key_rejects_unknown() {
        let mut a = base("key");
        a.key = Some("Meta".into());
        let err = validate_action(&a).unwrap_err();
        assert!(err.contains("unsupported key"));
    }

    #[test]
    fn navigate_deferred_to_host() {
        let mut a = base("navigate");
        a.url = Some("https://example.com".into());
        assert!(action_js(&a).unwrap_err().contains("host"));
    }

    #[test]
    fn wait_for_requires_predicate() {
        let a = base("waitFor");
        assert!(validate_action(&a).unwrap_err().contains("urlContains"));
    }

    #[test]
    fn wait_for_js() {
        let mut a = base("waitFor");
        a.url_contains = Some("example".into());
        a.timeout_ms = Some(5000);
        let js = action_js(&a).unwrap();
        assert!(js.contains("waitForCheck"));
        assert!(js.contains("example"));
    }

    #[test]
    fn unknown_kind_rejected() {
        assert!(validate_action(&base("teleport")).unwrap_err().contains("unknown"));
    }

    #[test]
    fn click_accepts_selector_without_xy() {
        let mut a = base("click");
        a.selector = Some("#save".into());
        assert!(validate_action(&a).is_ok());
        let js = action_js(&a).unwrap();
        assert!(js.contains("clickAt(null,null,\"t1\",\"#save\",null)"), "{js}");
    }

    #[test]
    fn click_accepts_ref_without_xy() {
        let mut a = base("click");
        a.ref_id = Some("e3".into());
        assert!(validate_action(&a).is_ok());
        let js = action_js(&a).unwrap();
        assert!(js.contains("\"e3\""), "{js}");
    }

    #[test]
    fn click_requires_target() {
        let a = base("click");
        assert!(validate_action(&a).unwrap_err().contains("x,y"));
    }

    #[test]
    fn fill_js_includes_target_and_clear() {
        let mut a = base("fill");
        a.text = Some("hello".into());
        a.selector = Some("#email".into());
        let js = action_js(&a).unwrap();
        assert!(
            js.contains("fillField(\"hello\",\"#email\",null,null,null,\"t1\",true)"),
            "{js}"
        );
    }

    #[test]
    fn fill_requires_text_and_target() {
        let mut a = base("fill");
        a.selector = Some("#x".into());
        assert!(validate_action(&a).is_err());
        a.text = Some("hi".into());
        assert!(validate_action(&a).is_ok());
        let mut b = base("fill");
        b.text = Some("hi".into());
        assert!(validate_action(&b).is_err());
    }
}

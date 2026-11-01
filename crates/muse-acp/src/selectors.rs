//! The bridge's model of the Muse web app's DOM.
//!
//! # Read this before trusting a turn
//!
//! The Muse web app has no public API or stable DOM contract, so this
//! module is a best-effort model, not a verified one. It was written from
//! the standard shape of a chat web app (a composer field, a send control,
//! a stream of assistant messages) and it is deliberately built to fail
//! loudly: every probe reports *which* candidate matched (or that none
//! did), and a turn that cannot find the composer or a response container
//! ends with a [`crate::BridgeError::WebDom`] naming what was looked for,
//! instead of returning an empty success.
//!
//! ## How to verify / fix
//!
//! Run the bridge headed (`MUSE_ACP_HEADLESS=0`) against a signed-in Muse
//! session, watch stderr for the `selector matched` diagnostics, and replace
//! the candidate lists below with the real `data-testid` / aria hooks the
//! app actually uses. The rest of the bridge (ACP framing, streaming,
//! cancel, auth marker) does not depend on these strings.
//!
//! Until that logged-in run happens, treat every successful-looking turn
//! from this driver as provisional.

/// The Muse web app entry point. The bridge navigates each new session's
/// tab here; the app routes to the current chat or a fresh one.
pub const MUSE_CHAT_URL: &str = "https://muse.ai/";

/// JavaScript probe evaluated on every poll during a turn.
///
/// Returns a JSON object:
/// ```json
/// {
///   "composer": "<matched selector or null>",
///   "sendEnabled": true,
///   "stopPresent": false,
///   "loginWall": false,
///   "responseMode": "candidate" | "fallback-main" | "none",
///   "responseSelector": "<matched selector or null>",
///   "responseCount": "<number of matching messages, or null in fallback mode>",
///   "responseText": "<current assistant text>"
/// }
/// ```
///
/// Candidate order is deliberate: explicit test hooks first, generic
/// roles/attributes second, a `main`-element fallback last (flagged as
/// `fallback-main` so a turn built on it is diagnosable, not silent).
pub const PROBE_JS: &str = r#"
(() => {
  const visible = (el) => {
    if (!el) return false;
    const r = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && style.visibility !== 'hidden' && style.display !== 'none';
  };
  const firstVisible = (selectorList) => {
    for (const sel of selectorList) {
      try {
        for (const el of document.querySelectorAll(sel)) {
          if (visible(el)) return { el, sel };
        }
      } catch (_) { /* invalid selector: try the next candidate */ }
    }
    return { el: null, sel: null };
  };
  const composer = firstVisible([
    '[data-testid*="composer"]',
    'textarea',
    '[contenteditable="true"]',
    'input[type="text"]'
  ]);
  const send = firstVisible([
    'button[data-testid*="send"]',
    'button[aria-label*="Send" i]',
    'button[type="submit"]'
  ]);
  const stop = firstVisible([
    'button[data-testid*="stop"]',
    'button[aria-label*="Stop" i]'
  ]);
  const hasPasswordField = !!document.querySelector('input[type="password"]');
  const messageCandidates = [
    '[data-testid*="assistant"]',
    '[data-testid*="message"]',
    '[class*="assistant"]'
  ];
  let responseText = '';
  let responseSelector = null;
  let responseMode = 'none';
  let responseCount = null;
  for (const sel of messageCandidates) {
    try {
      const matches = [...document.querySelectorAll(sel)].filter(visible);
      if (matches.length > 0) {
        responseText = matches[matches.length - 1].innerText || '';
        responseSelector = sel;
        responseMode = 'candidate';
        responseCount = matches.length;
        break;
      }
    } catch (_) { /* try the next candidate */ }
  }
  if (responseMode === 'none') {
    const main = document.querySelector('main');
    if (main) {
      responseText = main.innerText || '';
      responseMode = 'fallback-main';
    }
  }
  return JSON.stringify({
    composer: composer.sel,
    sendEnabled: !!(send.el && !send.el.disabled),
    stopPresent: !!stop.el,
    loginWall: !composer.el && hasPasswordField,
    responseMode,
    responseSelector,
    responseCount,
    responseText
  });
})()
"#;

/// JavaScript submit step: place `__MUSE_ACP_TEXT__` (a JSON string literal,
/// substituted by the caller) into the composer and send it.
///
/// Returns a JSON object: `{ "submitted": true, "via": "send-button" |
/// "enter" | "none", "composer": "<matched selector or null>" }`.
/// `"via": "none"` means the text was placed but no send control or Enter
/// path fired — the caller treats that as a DOM failure, not a success.
pub const SUBMIT_JS_TEMPLATE: &str = r#"
(() => {
  const text = __MUSE_ACP_TEXT__;
  const visible = (el) => {
    if (!el) return false;
    const r = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && style.visibility !== 'hidden' && style.display !== 'none';
  };
  const firstVisible = (selectorList) => {
    for (const sel of selectorList) {
      try {
        for (const el of document.querySelectorAll(sel)) {
          if (visible(el)) return { el, sel };
        }
      } catch (_) { /* invalid selector: try the next candidate */ }
    }
    return { el: null, sel: null };
  };
  const composer = firstVisible([
    '[data-testid*="composer"]',
    'textarea',
    '[contenteditable="true"]',
    'input[type="text"]'
  ]);
  if (!composer.el) return JSON.stringify({ submitted: false, via: 'none', composer: null });
  const el = composer.el;
  el.focus();
  if (el.isContentEditable) {
    document.execCommand('selectAll', false, null);
    document.execCommand('insertText', false, text);
    el.dispatchEvent(new InputEvent('input', { bubbles: true, data: text }));
  } else {
    const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value')
      || Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value');
    if (setter && setter.set) {
      setter.set.call(el, text);
    } else {
      el.value = text;
    }
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }
  const send = firstVisible([
    'button[data-testid*="send"]',
    'button[aria-label*="Send" i]',
    'button[type="submit"]'
  ]);
  if (send.el && !send.el.disabled) {
    send.el.click();
    return JSON.stringify({ submitted: true, via: 'send-button', composer: composer.sel });
  }
  const enter = new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true });
  el.dispatchEvent(enter);
  return JSON.stringify({ submitted: true, via: 'enter', composer: composer.sel });
})()
"#;

/// Build the submit expression for one prompt text.
pub fn submit_js(text: &str) -> String {
    let literal = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
    SUBMIT_JS_TEMPLATE.replace("__MUSE_ACP_TEXT__", &literal)
}

/// JavaScript best-effort "new chat" step, run when a session tab is
/// created. Non-fatal when the control is absent: the session then
/// continues in whatever conversation the app opened, which is still a
/// coherent Muse conversation (just not a fresh one).
pub const NEW_CHAT_JS: &str = r#"
(() => {
  const candidates = [
    'button[data-testid*="new-chat"]',
    'button[aria-label*="New chat" i]',
    'a[data-testid*="new-chat"]'
  ];
  for (const sel of candidates) {
    try {
      for (const el of document.querySelectorAll(sel)) {
        const r = el.getBoundingClientRect();
        if (r.width > 0 && r.height > 0) { el.click(); return JSON.stringify({ clicked: true, selector: sel }); }
      }
    } catch (_) { /* try the next candidate */ }
  }
  return JSON.stringify({ clicked: false, selector: null });
})()
"#;

/// JavaScript best-effort stop step, run when a turn is cancelled.
pub const STOP_JS: &str = r#"
(() => {
  const candidates = [
    'button[data-testid*="stop"]',
    'button[aria-label*="Stop" i]'
  ];
  for (const sel of candidates) {
    try {
      for (const el of document.querySelectorAll(sel)) {
        const r = el.getBoundingClientRect();
        if (r.width > 0 && r.height > 0) { el.click(); return JSON.stringify({ clicked: true, selector: sel }); }
      }
    } catch (_) { /* try the next candidate */ }
  }
  return JSON.stringify({ clicked: false, selector: null });
})()
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_js_embeds_escaped_text() {
        let js = submit_js("hello \"world\"\nline 2");
        assert!(js.contains("hello \\\"world\\\"\\nline 2"));
        assert!(!js.contains("__MUSE_ACP_TEXT__"));
    }
}

//! Observe event ring + page instrumentation script.

use std::collections::{HashMap, VecDeque};
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const RING_CAP: usize = 500;
const BODY_CAP: usize = 32_768;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserveEvent {
    pub id: u64,
    pub webview_label: String,
    pub kind: String,
    pub at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

/// One event drained from the in-page instrumentation queue.
#[derive(Debug, Clone, Deserialize)]
pub struct PageDrainEvent {
    #[serde(default)]
    pub id: u64,
    pub kind: String,
    #[serde(default, rename = "atMs")]
    pub at_ms: Option<u64>,
    #[serde(default)]
    pub payload: Option<Value>,
}

#[derive(Debug)]
pub struct BrowserObserveBuffer {
    next_id: Mutex<u64>,
    rings: Mutex<HashMap<String, VecDeque<ObserveEvent>>>,
    root: Mutex<Option<PathBuf>>,
    /// Last emitted main-frame nav per label: (url, title) for dedupe / title fill.
    last_nav: Mutex<HashMap<String, (String, Option<String>)>>,
    /// Last page-seq id successfully ingested per label (separate from host ring ids).
    page_cursors: Mutex<HashMap<String, u64>>,
    /// Serialize page→host flushes so watcher + Tauri poll do not double-ingest.
    page_flush_busy: AtomicBool,
}

impl Default for BrowserObserveBuffer {
    fn default() -> Self {
        Self {
            // Ids start at 1 so after_id=0 means "from the beginning".
            next_id: Mutex::new(1),
            rings: Mutex::new(HashMap::new()),
            root: Mutex::new(None),
            last_nav: Mutex::new(HashMap::new()),
            page_cursors: Mutex::new(HashMap::new()),
            page_flush_busy: AtomicBool::new(false),
        }
    }
}

impl BrowserObserveBuffer {
    pub fn set_root(&self, root: PathBuf) {
        if let Ok(mut slot) = self.root.lock() {
            *slot = Some(root);
        }
    }

    pub fn push(&self, webview_label: &str, kind: &str, payload: Option<Value>, at_ms: u64) {
        let id = {
            let Ok(mut next) = self.next_id.lock() else {
                return;
            };
            let id = *next;
            *next = next.saturating_add(1);
            id
        };
        let event = ObserveEvent {
            id,
            webview_label: webview_label.to_string(),
            kind: kind.to_string(),
            at_ms,
            payload: payload.map(truncate_payload),
        };
        if let Ok(mut rings) = self.rings.lock() {
            let ring = rings.entry(webview_label.to_string()).or_default();
            if ring.len() >= RING_CAP {
                ring.pop_front();
            }
            ring.push_back(event.clone());
        }
        self.mirror_jsonl(&event);
    }

    /// Main-frame nav only: drop same-URL repeats; allow one upgrade when title arrives.
    /// Returns true when an event was recorded.
    pub fn push_nav(
        &self,
        webview_label: &str,
        url: &str,
        title: Option<&str>,
        at_ms: u64,
    ) -> bool {
        let url = url.trim();
        if url.is_empty() || url == "about:blank" {
            return false;
        }
        let title = title
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        if let Ok(mut last) = self.last_nav.lock() {
            if let Some((prev_url, prev_title)) = last.get(webview_label) {
                if prev_url == url {
                    // Same URL: only emit again when title newly available.
                    if title.is_none()
                        || prev_title.as_ref() == title.as_ref()
                        || prev_title.is_some()
                    {
                        return false;
                    }
                }
            }
            last.insert(webview_label.to_string(), (url.to_string(), title.clone()));
        }
        self.push(
            webview_label,
            "nav",
            Some(serde_json::json!({ "url": url, "title": title })),
            at_ms,
        );
        true
    }

    pub fn poll(&self, webview_label: &str, after_id: u64, limit: usize) -> Vec<ObserveEvent> {
        let Ok(rings) = self.rings.lock() else {
            return Vec::new();
        };
        let Some(ring) = rings.get(webview_label) else {
            return Vec::new();
        };
        ring.iter()
            .filter(|e| e.id > after_id)
            .take(limit.max(1).min(200))
            .cloned()
            .collect()
    }

    pub fn clear(&self, webview_label: &str) {
        if let Ok(mut rings) = self.rings.lock() {
            rings.remove(webview_label);
        }
        if let Ok(mut last) = self.last_nav.lock() {
            last.remove(webview_label);
        }
        if let Ok(mut cursors) = self.page_cursors.lock() {
            cursors.remove(webview_label);
        }
    }

    pub fn clear_surface_prefix(&self, prefix: &str) {
        if let Ok(mut rings) = self.rings.lock() {
            rings.retain(|k, _| !k.starts_with(prefix) && k != prefix);
        }
        if let Ok(mut cursors) = self.page_cursors.lock() {
            cursors.retain(|k, _| !k.starts_with(prefix) && k != prefix);
        }
    }

    /// Page-queue cursor (in-page seq), not the host ring `after_id`.
    pub fn page_after_id(&self, webview_label: &str) -> u64 {
        self.page_cursors
            .lock()
            .ok()
            .and_then(|c| c.get(webview_label).copied())
            .unwrap_or(0)
    }

    /// Claim exclusive page→host flush. Pair with [`end_page_flush`].
    pub fn try_begin_page_flush(&self) -> bool {
        self.page_flush_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub fn end_page_flush(&self) {
        self.page_flush_busy.store(false, Ordering::Release);
    }

    /// Push page-drained events into the host ring / events.jsonl and advance the
    /// page cursor. `expected_after` must match the cursor used for the drain so
    /// concurrent flushes do not double-ingest.
    pub fn ingest_page_events(
        &self,
        webview_label: &str,
        expected_after: u64,
        events: &[PageDrainEvent],
        fallback_at_ms: u64,
    ) -> usize {
        let to_push: Vec<&PageDrainEvent> =
            events.iter().filter(|e| e.id > expected_after).collect();
        if to_push.is_empty() {
            return 0;
        }
        {
            let Ok(mut cursors) = self.page_cursors.lock() else {
                return 0;
            };
            let cursor = cursors.entry(webview_label.to_string()).or_insert(0);
            if *cursor != expected_after {
                return 0;
            }
            let max_id = to_push.iter().map(|e| e.id).max().unwrap_or(expected_after);
            *cursor = max_id;
        }
        for pe in to_push {
            self.push(
                webview_label,
                &pe.kind,
                pe.payload.clone(),
                pe.at_ms.unwrap_or(fallback_at_ms),
            );
        }
        events.iter().filter(|e| e.id > expected_after).count()
    }

    fn mirror_jsonl(&self, event: &ObserveEvent) {
        let Ok(root_guard) = self.root.lock() else {
            return;
        };
        let Some(root) = root_guard.as_ref() else {
            return;
        };
        let dir = root.join(&event.webview_label);
        if create_dir_all(&dir).is_err() {
            return;
        }
        let path = dir.join("events.jsonl");
        let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
            return;
        };
        if let Ok(line) = serde_json::to_string(event) {
            let _ = writeln!(file, "{line}");
        }
    }
}

fn truncate_payload(value: Value) -> Value {
    match value {
        Value::Object(mut map) => {
            if let Some(Value::String(body)) = map.get("body").cloned() {
                if body.len() > BODY_CAP {
                    let truncated: String = body.chars().take(BODY_CAP).collect();
                    map.insert(
                        "body".into(),
                        Value::String(format!(
                            "{truncated}\n…[truncated {} chars]",
                            body.len().saturating_sub(BODY_CAP)
                        )),
                    );
                    map.insert("bodyTruncated".into(), Value::Bool(true));
                }
            }
            Value::Object(map)
        }
        other => other,
    }
}


/// Whether an HTML `target` attribute should open a sibling tab (not same-frame).
pub fn is_new_tab_link_target(target: &str) -> bool {
    let t = target.trim().to_ascii_lowercase();
    if t.is_empty() || t == "_self" || t == "_parent" || t == "_top" {
        return false;
    }
    t == "_blank" || t == "_new" || !t.starts_with('_')
}

/// Resolve a window.open / href candidate against a base URL; https only.
pub fn resolve_new_tab_url(raw: &str, base: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "about:blank" {
        return None;
    }
    let parsed = match url::Url::parse(raw) {
        Ok(u) => u,
        Err(_) => url::Url::parse(base).ok()?.join(raw).ok()?,
    };
    if parsed.scheme() != "https" {
        return None;
    }
    Some(parsed.to_string())
}

/// Page script: console + fetch/XHR hooks + ghost-cursor helpers shared with Drive.
pub fn instrumentation_js(webview_label: &str, drive: bool) -> String {
    let label = serde_json::to_string(webview_label).unwrap_or_else(|_| "\"\"".into());
    let drive_flag = if drive { "true" } else { "false" };
    format!(
        r#"(function(){{
  if (window.__buzzBrowserAgent && window.__buzzBrowserAgent.label === {label}) {{
    window.__buzzBrowserAgent.setDrive({drive_flag});
    return 'already';
  }}
  var LABEL = {label};
  var DRIVE = {drive_flag};
  var q = [];
  var seq = 0;
  function push(kind, payload) {{
    q.push({{ id: ++seq, kind: kind, atMs: Date.now(), payload: payload || null }});
    if (q.length > 500) q.shift();
  }}
  function drain(afterId, limit) {{
    afterId = afterId || 0; limit = Math.min(Math.max(limit || 50, 1), 200);
    var out = [];
    for (var i = 0; i < q.length; i++) {{
      if (q[i].id > afterId) {{ out.push(q[i]); if (out.length >= limit) break; }}
    }}
    return JSON.stringify(out);
  }}
  // Host flush via eval_with_callback (not cookies). Size-capped so network
  // bodies up to BODY_CAP survive. Resets afterId when page seq restarted.
  function drainForHost(afterId, limit, maxChars) {{
    afterId = afterId || 0;
    if (seq > 0 && afterId > seq) afterId = 0;
    limit = Math.min(Math.max(limit || 25, 1), 100);
    maxChars = maxChars || 262144;
    var out = [];
    var size = 2;
    for (var i = 0; i < q.length; i++) {{
      if (q[i].id <= afterId) continue;
      var piece;
      try {{ piece = JSON.stringify(q[i]); }} catch (e) {{ continue; }}
      if (out.length > 0 && size + piece.length + 1 > maxChars) break;
      out.push(q[i]);
      size += piece.length + 1;
      if (out.length >= limit) break;
    }}
    return out;
  }}
  ['log','info','warn','error','debug'].forEach(function(level) {{
    var orig = console[level];
    console[level] = function() {{
      try {{
        var args = Array.prototype.slice.call(arguments).map(function(a) {{
          try {{ return typeof a === 'string' ? a : JSON.stringify(a); }} catch (e) {{ return String(a); }}
        }});
        push('console', {{ level: level, args: args }});
      }} catch (e) {{}}
      return orig.apply(console, arguments);
    }};
  }});
  function headersObject(h) {{
    var o = {{}};
    try {{
      if (!h) return o;
      if (typeof h.forEach === 'function') {{ h.forEach(function(v,k){{ o[k]=v; }}); return o; }}
      if (Array.isArray(h)) {{ h.forEach(function(pair){{ if(pair&&pair[0]) o[pair[0]]=pair[1]; }}); return o; }}
      Object.keys(h).forEach(function(k){{ o[k]=h[k]; }});
    }} catch (e) {{}}
    return o;
  }}
  if (window.fetch) {{
    var ofetch = window.fetch.bind(window);
    window.fetch = function(input, init) {{
      var url = typeof input === 'string' ? input : (input && input.url) || String(input);
      var method = (init && init.method) || (input && input.method) || 'GET';
      var reqBody = init && init.body != null ? String(init.body).slice(0, 32768) : null;
      return ofetch(input, init).then(function(res) {{
        var clone = res.clone();
        clone.text().then(function(text) {{
          push('network', {{
            phase: 'response', url: url, method: method,
            status: res.status, ok: res.ok,
            requestHeaders: headersObject(init && init.headers),
            responseHeaders: headersObject(res.headers),
            requestBody: reqBody,
            body: text.slice(0, 32768),
            bodyTruncated: text.length > 32768
          }});
        }}).catch(function() {{
          push('network', {{
            phase: 'response', url: url, method: method,
            status: res.status, ok: res.ok, body: null
          }});
        }});
        return res;
      }}).catch(function(err) {{
        push('network', {{ phase: 'error', url: url, method: method, error: String(err) }});
        throw err;
      }});
    }};
  }}
  (function() {{
    var XO = XMLHttpRequest.prototype.open;
    var XS = XMLHttpRequest.prototype.send;
    XMLHttpRequest.prototype.open = function(method, url) {{
      this.__buzz = {{ method: method, url: String(url) }};
      return XO.apply(this, arguments);
    }};
    XMLHttpRequest.prototype.send = function(body) {{
      var self = this;
      var meta = self.__buzz || {{}};
      var reqBody = body != null ? String(body).slice(0, 32768) : null;
      self.addEventListener('loadend', function() {{
        push('network', {{
          phase: 'xhr', url: meta.url, method: meta.method,
          status: self.status, ok: self.status >= 200 && self.status < 400,
          requestBody: reqBody,
          body: (self.responseText || '').slice(0, 32768),
          bodyTruncated: (self.responseText || '').length > 32768
        }});
      }});
      return XS.apply(this, arguments);
    }};
  }})();
  function pushNavIfChanged(url, title) {{
    url = String(url || location.href);
    title = title == null ? document.title : title;
    var last = window.__buzzBrowserAgentNav;
    if (last && last.url === url && (last.title || '') === (title || '')) return;
    window.__buzzBrowserAgentNav = {{ url: url, title: title || '' }};
    push('nav', {{ url: url, title: title || null }});
  }}
  function resolveOpenUrl(raw) {{
    raw = String(raw == null ? '' : raw).trim();
    if (!raw || raw === 'about:blank') return '';
    try {{ return new URL(raw, location.href).href; }} catch (e) {{ return ''; }}
  }}
  function isNewTabTarget(target) {{
    target = String(target || '').trim().toLowerCase();
    if (!target || target === '_self' || target === '_parent' || target === '_top') return false;
    return target === '_blank' || target === '_new' || target.charAt(0) !== '_';
  }}
  function requestNewTab(rawUrl, reason) {{
    var href = resolveOpenUrl(rawUrl);
    if (!href) return false;
    try {{
      var u = new URL(href);
      if (u.protocol !== 'https:') return false;
    }} catch (e) {{ return false; }}
    push('new_tab', {{ url: href, reason: reason || 'open' }});
    return true;
  }}
  function newTabLinkFrom(el) {{
    if (!el || !el.closest) return null;
    var link = el.closest('a[href]');
    if (!link) return null;
    if (isNewTabTarget(link.getAttribute('target'))) return link;
    return null;
  }}
  // Top-level only (no iframe hooks). Initial nav comes from host page-load.
  window.addEventListener('hashchange', function() {{
    pushNavIfChanged(location.href, document.title);
  }});
  // Soft SPA transitions (pushState/replaceState) — host screenshots + observe.
  (function() {{
    var wrap = function(type) {{
      var orig = history[type];
      if (typeof orig !== 'function') return;
      history[type] = function() {{
        var ret = orig.apply(this, arguments);
        try {{ pushNavIfChanged(location.href, document.title); }} catch (e) {{}}
        return ret;
      }};
    }};
    wrap('pushState');
    wrap('replaceState');
    window.addEventListener('popstate', function() {{
      pushNavIfChanged(location.href, document.title);
    }});
  }})();
  // window.open → sibling tab via host (same path as target=_blank).
  (function() {{
    var origOpen = window.open;
    window.open = function(url, name, features) {{
      if (requestNewTab(url, 'window.open')) {{
        return null;
      }}
      if (typeof origOpen === 'function') {{
        try {{ return origOpen.apply(window, arguments); }} catch (e) {{ return null; }}
      }}
      return null;
    }};
  }})();
  var cursor = document.getElementById('__buzz_agent_cursor');
  if (!cursor) {{
    cursor = document.createElement('div');
    cursor.id = '__buzz_agent_cursor';
    cursor.style.cssText = 'position:fixed;z-index:2147483646;width:18px;height:18px;margin:-9px 0 0 -9px;border:2px solid #ff6b00;border-radius:50%;background:rgba(255,107,0,0.25);pointer-events:none;display:none;transition:left 400ms ease-out,top 400ms ease-out,opacity 120ms,transform 120ms;';
    document.documentElement.appendChild(cursor);
  }}
  var lock = document.getElementById('__buzz_agent_lock');
  if (!lock) {{
    lock = document.createElement('div');
    lock.id = '__buzz_agent_lock';
    lock.style.cssText = 'position:fixed;inset:0;z-index:2147483645;display:none;cursor:not-allowed;background:transparent;';
    document.documentElement.appendChild(lock);
  }}
  function setDrive(on) {{
    DRIVE = !!on;
    lock.style.display = DRIVE ? 'block' : 'none';
    cursor.style.display = DRIVE ? 'block' : 'none';
  }}
  // Block human scroll while Drive lock is up. Agent scrollBy/pressKey stay
  // allowed (programmatic / !isTrusted). Take control calls setDrive(false).
  function blockHumanScroll(e) {{
    if (!DRIVE) return;
    if (e && e.isTrusted === false) return;
    try {{ e.preventDefault(); }} catch (err) {{}}
    try {{ e.stopPropagation(); }} catch (err) {{}}
  }}
  function blockHumanScrollKey(e) {{
    if (!DRIVE) return;
    if (e && e.isTrusted === false) return;
    var k = e.key || e.code;
    if (
      k === ' ' || k === 'Spacebar' || k === 'Space' ||
      k === 'PageUp' || k === 'PageDown' ||
      k === 'ArrowUp' || k === 'ArrowDown' ||
      k === 'Home' || k === 'End'
    ) {{
      try {{ e.preventDefault(); }} catch (err) {{}}
      try {{ e.stopPropagation(); }} catch (err) {{}}
    }}
  }}
  document.addEventListener('wheel', blockHumanScroll, {{ capture: true, passive: false }});
  document.addEventListener('touchmove', blockHumanScroll, {{ capture: true, passive: false }});
  document.addEventListener('keydown', blockHumanScrollKey, true);
  lock.addEventListener('wheel', blockHumanScroll, {{ passive: false }});
  lock.addEventListener('touchmove', blockHumanScroll, {{ passive: false }});
  // Human click/input/keydown → Observe events (tag/name/ref + valueLength only).
  // Never emit field values (PHI / passwords). Skip agent-synthetic (!isTrusted).
  function resolveUserRef(el) {{
    if (!el) return undefined;
    try {{
      var last = window.__buzzSnapshotLast;
      if (typeof last === 'string') last = JSON.parse(last);
      var list = last && last.interactives ? last.interactives : [];
      if (!list.length || !el.getBoundingClientRect) return undefined;
      var rect = el.getBoundingClientRect();
      var cx = Math.round(rect.left + rect.width / 2);
      var cy = Math.round(rect.top + rect.height / 2);
      for (var i = 0; i < list.length; i++) {{
        var it = list[i];
        if (!it || !it.center || !it.ref) continue;
        if (Math.abs(it.center.x - cx) <= 3 && Math.abs(it.center.y - cy) <= 3) return it.ref;
      }}
    }} catch (e) {{}}
    return undefined;
  }}
  function describeUserHit(el) {{
    if (!el || isAgentChrome(el)) return null;
    var tag = (el.tagName || '').toLowerCase();
    var role = el.getAttribute && el.getAttribute('role');
    var name = '';
    if (el.getAttribute) {{
      name = el.getAttribute('aria-label') || el.getAttribute('name') || el.getAttribute('title') || el.getAttribute('placeholder') || el.id || '';
    }}
    // Never use el.value as name (would leak PHI / password contents).
    if (!name && el.innerText && tag !== 'input' && tag !== 'textarea') {{
      name = String(el.innerText).trim().slice(0, 80);
    }}
    var out = {{ tag: tag }};
    if (role) out.role = role;
    if (name) out.name = String(name).slice(0, 80);
    var ref = resolveUserRef(el);
    if (ref) out.ref = ref;
    return out;
  }}
  function valueLengthOf(el) {{
    if (!el) return undefined;
    try {{
      if ('value' in el && el.value != null) return String(el.value).length;
      if (el.isContentEditable) return String(el.innerText || el.textContent || '').length;
    }} catch (e) {{}}
    return undefined;
  }}
  function userEventPayload(el, extra) {{
    var hit = describeUserHit(el);
    if (!hit) return null;
    var payload = {{ tag: hit.tag }};
    if (hit.role) payload.role = hit.role;
    if (hit.name) payload.name = hit.name;
    if (hit.ref) payload.ref = hit.ref;
    var vl = valueLengthOf(el);
    if (typeof vl === 'number') payload.valueLength = vl;
    if (extra) {{ for (var k in extra) {{ if (extra[k] != null) payload[k] = extra[k]; }} }}
    // Never attach a `value` field.
    return payload;
  }}
  function onUserClick(e) {{
    if (!e || e.isTrusted === false) return;
    try {{
      var payload = userEventPayload(e.target);
      if (payload) push('user_click', payload);
    }} catch (err) {{}}
  }}
  function onUserInput(e) {{
    if (!e || e.isTrusted === false) return;
    try {{
      var payload = userEventPayload(e.target);
      if (payload) push('user_input', payload);
    }} catch (err) {{}}
  }}
  function onUserKeydown(e) {{
    if (!e || e.isTrusted === false) return;
    try {{
      var extra = {{}};
      var k = e.key || '';
      // Emit key only for non-printable / modified keys — never typed characters.
      if (k.length > 1 || e.ctrlKey || e.metaKey || e.altKey) {{
        extra.key = k;
      }} else if (k === ' ') {{
        extra.key = ' ';
      }}
      var payload = userEventPayload(e.target, extra);
      if (payload) push('user_keydown', payload);
    }} catch (err) {{}}
  }}
  document.addEventListener('click', onUserClick, true);
  document.addEventListener('input', onUserInput, true);
  document.addEventListener('keydown', onUserKeydown, true);
  var cursorChain = Promise.resolve();
  function theaterFast() {{
    try {{
      if (window.__buzzDriveFast === true) return true;
      if (typeof document.hidden === 'boolean' && document.hidden) return true;
    }} catch (e) {{}}
    return false;
  }}
  function moveCursor(x, y, flash) {{
    if (theaterFast()) {{
      cursor.style.transition = 'none';
      cursor.style.left = x + 'px';
      cursor.style.top = y + 'px';
      cursor.style.display = 'block';
      if (flash) {{
        cursor.style.opacity = '1';
        cursor.style.transform = 'scale(1.15)';
        setTimeout(function(){{ cursor.style.transform = 'scale(1)'; }}, 40);
      }}
      return Promise.resolve();
    }}
    var dur = 300 + Math.floor(Math.random() * 301);
    cursor.style.transition = 'left ' + dur + 'ms ease-out, top ' + dur + 'ms ease-out, opacity 120ms, transform 120ms';
    cursor.style.left = x + 'px';
    cursor.style.top = y + 'px';
    cursor.style.display = 'block';
    if (flash) {{
      cursor.style.opacity = '1';
      cursor.style.transform = 'scale(1.4)';
      setTimeout(function(){{ cursor.style.transform = 'scale(1)'; }}, 120);
    }}
    return new Promise(function(resolve) {{
      setTimeout(resolve, dur + (flash ? 140 : 0));
    }});
  }}
  function enqueueAction(fn) {{
    var run = cursorChain.then(function() {{ return fn(); }});
    cursorChain = run.catch(function(){{}});
    return run;
  }}
  function isAgentChrome(el) {{
    if (!el || !el.id) return false;
    return el.id === '__buzz_agent_lock' || el.id === '__buzz_agent_cursor';
  }}
  var INTERACTIVE_SEL = 'a[href],button,input,select,textarea,[role="button"],[role="link"],[role="textbox"],[tabindex]:not([tabindex="-1"])';
  function isHitVisible(el) {{
    if (!el || !el.getBoundingClientRect) return false;
    var rect = el.getBoundingClientRect();
    if (rect.width < 1 || rect.height < 1) return false;
    try {{
      var style = window.getComputedStyle(el);
      if (style && (style.visibility === 'hidden' || style.display === 'none' || style.opacity === '0')) return false;
    }} catch (e) {{}}
    return true;
  }}
  // Layout-based hit when elementsFromPoint is empty (hidden/parked WKWebView)
  // or only returns agent chrome. Matches snapshot centers from getBoundingClientRect.
  function hitTestGeometry(x, y) {{
    var best = null;
    var bestArea = Infinity;
    function consider(el) {{
      if (!el || isAgentChrome(el) || !isHitVisible(el)) return;
      var rect = el.getBoundingClientRect();
      if (x < rect.left || x > rect.right || y < rect.top || y > rect.bottom) return;
      var area = rect.width * rect.height;
      if (area < bestArea) {{
        best = el;
        bestArea = area;
      }}
    }}
    try {{
      var nodes = document.querySelectorAll(INTERACTIVE_SEL);
      for (var i = 0; i < nodes.length; i++) consider(nodes[i]);
    }} catch (e) {{}}
    if (best) return best;
    try {{
      var all = document.body ? document.body.getElementsByTagName('*') : [];
      for (var j = 0; j < all.length; j++) consider(all[j]);
    }} catch (e2) {{}}
    return best;
  }}
  function hitTest(x, y) {{
    // Pierce the Drive lock for the duration of the hit-test. The overlay must
    // still swallow human clicks (pointer-events restored immediately after).
    var prevPe = null;
    if (lock && lock.style) {{
      prevPe = lock.style.pointerEvents;
      lock.style.pointerEvents = 'none';
    }}
    var els = [];
    try {{
      if (typeof document.elementsFromPoint === 'function') {{
        els = document.elementsFromPoint(x, y) || [];
      }} else {{
        var one = document.elementFromPoint(x, y);
        if (one) els = [one];
      }}
    }} catch (e) {{ els = []; }}
    if (lock && lock.style) {{
      lock.style.pointerEvents = prevPe || '';
    }}
    for (var i = 0; i < els.length; i++) {{
      if (!isAgentChrome(els[i])) return els[i];
    }}
    return hitTestGeometry(x, y);
  }}
  function describeHit(el) {{
    if (!el) return null;
    var tag = (el.tagName || '').toLowerCase();
    var role = el.getAttribute && el.getAttribute('role');
    var name = (el.getAttribute && (el.getAttribute('aria-label') || el.getAttribute('name') || el.getAttribute('title'))) || '';
    if (!name && el.innerText) name = String(el.innerText).trim().slice(0, 80);
    if (!name && el.value != null && (tag === 'input' || tag === 'textarea')) name = String(el.value).slice(0, 80);
    return {{ tag: tag, role: role || undefined, name: name || undefined }};
  }}
  function isFocusable(el) {{
    if (!el || el.disabled) return false;
    var tag = (el.tagName || '').toLowerCase();
    if (tag === 'input' || tag === 'select' || tag === 'textarea' || tag === 'button') return true;
    if (tag === 'a' && el.href) return true;
    if (el.isContentEditable) return true;
    if (typeof el.tabIndex === 'number' && el.tabIndex >= 0) return true;
    return false;
  }}
  function makeResult(id, kind, ok, hit, error) {{
    var r = {{ id: id, ok: !!ok, kind: kind, url: location.href }};
    if (hit) r.hit = hit;
    if (error) r.error = String(error);
    return JSON.stringify(r);
  }}
  function dispatchPointer(el, type, x, y, extra) {{
    var opts = {{
      bubbles: true, cancelable: true, composed: true,
      clientX: x, clientY: y, view: window,
      pointerId: 1, pointerType: 'mouse', isPrimary: true
    }};
    if (extra) {{ for (var k in extra) opts[k] = extra[k]; }}
    try {{
      if (typeof PointerEvent === 'function') {{
        el.dispatchEvent(new PointerEvent(type, opts));
        return;
      }}
    }} catch (e) {{}}
  }}
  function setNativeValue(el, value) {{
    var proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    var desc = Object.getOwnPropertyDescriptor(proto, 'value');
    if (desc && desc.set) desc.set.call(el, value);
    else el.value = value;
  }}
  function resolveTarget(x, y, selector, ref) {{
    if (selector) {{
      try {{
        var bySel = document.querySelector(selector);
        if (bySel) return bySel;
      }} catch (e) {{}}
    }}
    if (ref) {{
      try {{
        var last = window.__buzzSnapshotLast;
        if (typeof last === 'string') last = JSON.parse(last);
        var list = last && last.interactives ? last.interactives : [];
        for (var i = 0; i < list.length; i++) {{
          if (list[i] && list[i].ref === ref && list[i].center) {{
            return hitTest(list[i].center.x, list[i].center.y) || hitTestGeometry(list[i].center.x, list[i].center.y);
          }}
        }}
      }} catch (e2) {{}}
    }}
    if (typeof x === 'number' && typeof y === 'number') return hitTest(x, y);
    return null;
  }}
  function centerOf(el, x, y) {{
    if (typeof x === 'number' && typeof y === 'number') return {{ x: x, y: y }};
    var rect = el.getBoundingClientRect();
    return {{ x: Math.round(rect.left + rect.width / 2), y: Math.round(rect.top + rect.height / 2) }};
  }}
  function clickAt(x, y, id, selector, ref) {{
    id = id || ('d' + Date.now());
    return enqueueAction(async function() {{
      var el = resolveTarget(x, y, selector, ref);
      if (!el) return makeResult(id, 'click', false, null, 'no element');
      var pt = centerOf(el, x, y);
      var hit = describeHit(el);
      var newTabLink = newTabLinkFrom(el);
      // Theater cursor runs async; ack as soon as DOM click fires.
      try {{ moveCursor(pt.x, pt.y, true); }} catch (eCursor) {{}}
      try {{
        dispatchPointer(el, 'pointerover', pt.x, pt.y, {{ buttons: 0 }});
        el.dispatchEvent(new MouseEvent('mouseover', {{ bubbles:true, cancelable:true, clientX:pt.x, clientY:pt.y, view:window }}));
        dispatchPointer(el, 'pointerdown', pt.x, pt.y, {{ buttons: 1 }});
        el.dispatchEvent(new MouseEvent('mousedown', {{ bubbles:true, cancelable:true, clientX:pt.x, clientY:pt.y, buttons:1, view:window }}));
        if (isFocusable(el) && typeof el.focus === 'function') {{
          try {{ el.focus(); }} catch (e) {{}}
        }}
        dispatchPointer(el, 'pointerup', pt.x, pt.y, {{ buttons: 0 }});
        el.dispatchEvent(new MouseEvent('mouseup', {{ bubbles:true, cancelable:true, clientX:pt.x, clientY:pt.y, view:window }}));
        // target=_blank / named targets: synthetic clicks are untrusted and
        // often skip WKWebView createWebView — ask host to open a sibling tab.
        if (newTabLink) {{
          requestNewTab(newTabLink.href, 'target=_blank');
        }} else {{
          el.dispatchEvent(new MouseEvent('click', {{ bubbles:true, cancelable:true, clientX:pt.x, clientY:pt.y, view:window }}));
          // Prefer native click() for same-frame <a> default navigation.
          try {{
            if (el.closest && el.closest('a[href]') && typeof el.click === 'function') {{
              el.click();
            }}
          }} catch (e2) {{}}
        }}
      }} catch (e) {{
        return makeResult(id, 'click', false, hit, e);
      }}
      return makeResult(id, 'click', true, hit, null);
    }});
  }}
  function hoverAt(x, y, id, selector, ref) {{
    id = id || ('d' + Date.now());
    return enqueueAction(async function() {{
      var el = resolveTarget(x, y, selector, ref);
      if (!el) return makeResult(id, 'hover', false, null, 'no element');
      var pt = centerOf(el, x, y);
      var hit = describeHit(el);
      try {{ moveCursor(pt.x, pt.y, false); }} catch (eCursor) {{}}
      try {{
        dispatchPointer(el, 'pointerover', pt.x, pt.y, {{ buttons: 0 }});
        el.dispatchEvent(new MouseEvent('mouseover', {{ bubbles:true, cancelable:true, clientX:pt.x, clientY:pt.y, view:window }}));
        el.dispatchEvent(new MouseEvent('mousemove', {{ bubbles:true, cancelable:true, clientX:pt.x, clientY:pt.y, view:window }}));
      }} catch (e) {{
        return makeResult(id, 'hover', false, hit, e);
      }}
      return makeResult(id, 'hover', true, hit, null);
    }});
  }}
  function typeText(text, selector, id) {{
    id = id || ('d' + Date.now());
    text = String(text == null ? '' : text);
    return enqueueAction(async function() {{
      var el = selector ? document.querySelector(selector) : document.activeElement;
      if (!el) return makeResult(id, 'type', false, null, 'no target');
      try {{ el.focus(); }} catch (e) {{}}
      var hit = describeHit(el);
      var delay = theaterFast() ? 0 : (30 + Math.floor(Math.random() * 31));
      try {{
        if ('value' in el) {{
          var base = el.value || '';
          for (var i = 0; i < text.length; i++) {{
            base = base + text.charAt(i);
            setNativeValue(el, base);
            el.dispatchEvent(new Event('input', {{ bubbles:true }}));
            try {{
              el.dispatchEvent(new InputEvent('input', {{ bubbles:true, data: text.charAt(i), inputType: 'insertText' }}));
            }} catch (e) {{}}
            if (delay > 0) await new Promise(function(r) {{ setTimeout(r, delay); }});
          }}
          el.dispatchEvent(new Event('change', {{ bubbles:true }}));
        }} else if (el.isContentEditable) {{
          for (var j = 0; j < text.length; j++) {{
            el.textContent = (el.textContent || '') + text.charAt(j);
            el.dispatchEvent(new Event('input', {{ bubbles:true }}));
            if (delay > 0) await new Promise(function(r) {{ setTimeout(r, delay); }});
          }}
        }} else {{
          return makeResult(id, 'type', false, hit, 'target not editable');
        }}
      }} catch (e) {{
        return makeResult(id, 'type', false, hit, e);
      }}
      return makeResult(id, 'type', true, hit, null);
    }});
  }}
  function fillField(text, selector, ref, x, y, id, clear) {{
    id = id || ('d' + Date.now());
    text = String(text == null ? '' : text);
    clear = clear !== false;
    return enqueueAction(async function() {{
      var el = resolveTarget(x, y, selector, ref);
      if (!el) return makeResult(id, 'fill', false, null, 'no element');
      var pt = centerOf(el, x, y);
      var hit = describeHit(el);
      try {{ moveCursor(pt.x, pt.y, true); }} catch (eCursor) {{}}
      try {{
        dispatchPointer(el, 'pointerover', pt.x, pt.y, {{ buttons: 0 }});
        el.dispatchEvent(new MouseEvent('mouseover', {{ bubbles:true, cancelable:true, clientX:pt.x, clientY:pt.y, view:window }}));
        dispatchPointer(el, 'pointerdown', pt.x, pt.y, {{ buttons: 1 }});
        el.dispatchEvent(new MouseEvent('mousedown', {{ bubbles:true, cancelable:true, clientX:pt.x, clientY:pt.y, buttons:1, view:window }}));
        if (isFocusable(el) && typeof el.focus === 'function') {{
          try {{ el.focus(); }} catch (e) {{}}
        }}
        dispatchPointer(el, 'pointerup', pt.x, pt.y, {{ buttons: 0 }});
        el.dispatchEvent(new MouseEvent('mouseup', {{ bubbles:true, cancelable:true, clientX:pt.x, clientY:pt.y, view:window }}));
        el.dispatchEvent(new MouseEvent('click', {{ bubbles:true, cancelable:true, clientX:pt.x, clientY:pt.y, view:window }}));
      }} catch (eClick) {{
        return makeResult(id, 'fill', false, hit, eClick);
      }}
      var delay = theaterFast() ? 0 : (20 + Math.floor(Math.random() * 21));
      try {{
        if ('value' in el) {{
          if (clear) {{
            setNativeValue(el, '');
            el.dispatchEvent(new Event('input', {{ bubbles:true }}));
          }}
          var base = clear ? '' : String(el.value || '');
          for (var i = 0; i < text.length; i++) {{
            base = base + text.charAt(i);
            setNativeValue(el, base);
            el.dispatchEvent(new Event('input', {{ bubbles:true }}));
            try {{
              el.dispatchEvent(new InputEvent('input', {{ bubbles:true, data: text.charAt(i), inputType: 'insertText' }}));
            }} catch (e) {{}}
            if (delay > 0) await new Promise(function(r) {{ setTimeout(r, delay); }});
          }}
          el.dispatchEvent(new Event('change', {{ bubbles:true }}));
          var actual = String(el.value || '');
          if (clear) {{
            if (actual !== text) {{
              return makeResult(id, 'fill', false, hit, 'verify failed: got ' + JSON.stringify(actual.slice(0, 80)));
            }}
          }} else if (!actual.endsWith(text)) {{
            return makeResult(id, 'fill', false, hit, 'verify failed: got ' + JSON.stringify(actual.slice(0, 80)));
          }}
        }} else if (el.isContentEditable) {{
          if (clear) {{
            el.textContent = '';
            el.dispatchEvent(new Event('input', {{ bubbles:true }}));
          }}
          var cur = clear ? '' : String(el.textContent || '');
          for (var j = 0; j < text.length; j++) {{
            cur = cur + text.charAt(j);
            el.textContent = cur;
            el.dispatchEvent(new Event('input', {{ bubbles:true }}));
            if (delay > 0) await new Promise(function(r) {{ setTimeout(r, delay); }});
          }}
          var got = String(el.textContent || '');
          if (clear) {{
            if (got !== text) {{
              return makeResult(id, 'fill', false, hit, 'verify failed: got ' + JSON.stringify(got.slice(0, 80)));
            }}
          }} else if (!got.endsWith(text)) {{
            return makeResult(id, 'fill', false, hit, 'verify failed: got ' + JSON.stringify(got.slice(0, 80)));
          }}
        }} else {{
          return makeResult(id, 'fill', false, hit, 'target not editable');
        }}
      }} catch (e) {{
        return makeResult(id, 'fill', false, hit, e);
      }}
      return makeResult(id, 'fill', true, hit, null);
    }});
  }}
  function scrollBy(dx, dy, id) {{
    id = id || ('d' + Date.now());
    return enqueueAction(async function() {{
      window.scrollBy(dx || 0, dy || 0);
      return makeResult(id, 'scroll', true, null, null);
    }});
  }}
  function keyCodeFor(key) {{
    var map = {{
      Enter: 'Enter', Tab: 'Tab', Escape: 'Escape', Backspace: 'Backspace',
      ArrowLeft: 'ArrowLeft', ArrowRight: 'ArrowRight', ArrowUp: 'ArrowUp', ArrowDown: 'ArrowDown'
    }};
    return map[key] || key;
  }}
  function keyOpts(key) {{
    return {{
      key: key, code: keyCodeFor(key), bubbles: true, cancelable: true,
      composed: true, view: window
    }};
  }}
  function focusables() {{
    var nodes = document.querySelectorAll('a[href],button,input,select,textarea,[tabindex]:not([tabindex="-1"])');
    var out = [];
    for (var i = 0; i < nodes.length; i++) {{
      var n = nodes[i];
      if (n.disabled || n.getAttribute('aria-hidden') === 'true') continue;
      if (isAgentChrome(n)) continue;
      out.push(n);
    }}
    return out;
  }}
  function moveTab(shift) {{
    var list = focusables();
    if (!list.length) return;
    var cur = document.activeElement;
    var idx = list.indexOf(cur);
    var next;
    if (shift) {{
      next = idx <= 0 ? list[list.length - 1] : list[idx - 1];
    }} else {{
      next = (idx < 0 || idx >= list.length - 1) ? list[0] : list[idx + 1];
    }}
    try {{ next.focus(); }} catch (e) {{}}
  }}
  function pressKey(key, id) {{
    id = id || ('d' + Date.now());
    key = String(key || '');
    return enqueueAction(async function() {{
    var el = document.activeElement || document.body;
    var hit = describeHit(el);
    var opts = keyOpts(key);
    try {{
      var kd = new KeyboardEvent('keydown', opts);
      var prevented = !el.dispatchEvent(kd) || kd.defaultPrevented;
      if (!prevented) {{
        try {{ el.dispatchEvent(new KeyboardEvent('keypress', opts)); }} catch (e) {{}}
        if (key === 'Enter') {{
          var tag = (el.tagName || '').toLowerCase();
          if (tag === 'textarea') {{
            if ('value' in el) {{
              var start = el.selectionStart != null ? el.selectionStart : el.value.length;
              var end = el.selectionEnd != null ? el.selectionEnd : start;
              setNativeValue(el, el.value.slice(0, start) + '\n' + el.value.slice(end));
              try {{ el.selectionStart = el.selectionEnd = start + 1; }} catch (e) {{}}
              el.dispatchEvent(new Event('input', {{ bubbles:true }}));
            }}
          }} else {{
            var form = el.form || (el.closest && el.closest('form'));
            if (form && typeof form.requestSubmit === 'function') {{
              try {{ form.requestSubmit(); }} catch (e) {{
                try {{ form.submit(); }} catch (e2) {{}}
              }}
            }} else if (tag === 'button' || (tag === 'input' && /submit|button/i.test(el.type || ''))) {{
              try {{ el.click(); }} catch (e) {{}}
            }}
          }}
        }} else if (key === 'Tab') {{
          moveTab(false);
        }} else if (key === 'Backspace') {{
          if ('value' in el && typeof el.value === 'string') {{
            var s = el.selectionStart != null ? el.selectionStart : el.value.length;
            var epos = el.selectionEnd != null ? el.selectionEnd : s;
            if (s !== epos) {{
              setNativeValue(el, el.value.slice(0, s) + el.value.slice(epos));
              try {{ el.selectionStart = el.selectionEnd = s; }} catch (e) {{}}
            }} else if (s > 0) {{
              setNativeValue(el, el.value.slice(0, s - 1) + el.value.slice(s));
              try {{ el.selectionStart = el.selectionEnd = s - 1; }} catch (e) {{}}
            }}
            el.dispatchEvent(new Event('input', {{ bubbles:true }}));
          }} else if (el.isContentEditable) {{
            document.execCommand && document.execCommand('delete');
          }}
        }} else if (key === 'Escape') {{
          // no special-case site logic; page handlers already got keydown
        }}
      }}
      el.dispatchEvent(new KeyboardEvent('keyup', opts));
    }} catch (e) {{
      return makeResult(id, 'key', false, hit, e);
    }}
    return makeResult(id, 'key', true, describeHit(document.activeElement || el), null);
    }});
  }}
  function waitForCheck(urlContains, selector, text, id, _timeoutMs) {{
    id = id || ('d' + Date.now());
    try {{
      if (urlContains && String(location.href).indexOf(urlContains) === -1) {{
        return makeResult(id, 'waitFor', false, null, 'urlContains not matched');
      }}
      if (selector) {{
        var el = document.querySelector(selector);
        if (!el) return makeResult(id, 'waitFor', false, null, 'selector not found');
      }}
      if (text) {{
        var bodyText = (document.body && (document.body.innerText || document.body.textContent)) || '';
        if (bodyText.indexOf(text) === -1) {{
          return makeResult(id, 'waitFor', false, null, 'text not found');
        }}
      }}
      return makeResult(id, 'waitFor', true, null, null);
    }} catch (e) {{
      return makeResult(id, 'waitFor', false, null, e);
    }}
  }}
  function snapshotCollect() {{
    var focused = document.activeElement ? describeHit(document.activeElement) : null;
    var interactives = [];
    var nodes = document.querySelectorAll(INTERACTIVE_SEL);
    for (var i = 0; i < nodes.length && interactives.length < 80; i++) {{
      var n = nodes[i];
      if (isAgentChrome(n)) continue;
      var rect = n.getBoundingClientRect();
      if (rect.width < 1 || rect.height < 1) continue;
      if (rect.bottom < 0 || rect.right < 0 || rect.top > innerHeight || rect.left > innerWidth) continue;
      var style = window.getComputedStyle(n);
      if (style && (style.visibility === 'hidden' || style.display === 'none' || style.opacity === '0')) continue;
      var hit = describeHit(n);
      var value = null;
      if ('value' in n && n.value != null && String(n.value).length) value = String(n.value).slice(0, 120);
      interactives.push({{
        ref: 'e' + interactives.length,
        tag: hit.tag, role: hit.role, name: hit.name, value: value,
        center: {{ x: Math.round(rect.left + rect.width / 2), y: Math.round(rect.top + rect.height / 2) }}
      }});
    }}
    return JSON.stringify({{
      url: location.href,
      title: document.title,
      viewport: {{ width: innerWidth, height: innerHeight, dpr: window.devicePixelRatio || 1 }},
      scroll: {{ x: window.scrollX, y: window.scrollY }},
      focused: focused,
      interactives: interactives
    }});
  }}
  window.__buzzBrowserAgent = {{
    label: LABEL,
    push: push,
    pushNavIfChanged: pushNavIfChanged,
    drain: drain,
    drainForHost: drainForHost,
    setDrive: setDrive,
    moveCursor: moveCursor,
    clickAt: clickAt,
    hoverAt: hoverAt,
    typeText: typeText,
    fillField: fillField,
    scrollBy: scrollBy,
    pressKey: pressKey,
    waitForCheck: waitForCheck,
    hitTest: hitTest,
    snapshotCollect: snapshotCollect
  }};
  setDrive(DRIVE);
  return 'ok';
}})();"#
    )
}

/// JS for `eval_with_callback`: returns a JSON array of page events (not a cookie).
/// Prefer `drainForHost` (size-capped); fall back to parsing string `drain`.
pub fn drain_page_queue_js(after_id: u64, limit: usize) -> String {
    format!(
        r#"(function(){{
  var agent = window.__buzzBrowserAgent;
  if (!agent) return [];
  try {{
    if (agent.drainForHost) return agent.drainForHost({after_id}, {limit}, 262144);
    var raw = agent.drain({after_id}, {limit});
    return typeof raw === 'string' ? JSON.parse(raw) : (raw || []);
  }} catch (e) {{ return []; }}
}})();"#
    )
}

pub const SNAPSHOT_COOKIE: &str = "__buzz_ba_snapshot";

/// Collect a11y/DOM snapshot and return JSON for `eval_with_callback`.
/// Also mirrors into `__buzzSnapshotLast` (and a best-effort cookie for legacy).
pub fn snapshot_collect_js() -> String {
    r#"(function(){
  var agent = window.__buzzBrowserAgent;
  if (!agent || !agent.snapshotCollect) {
    return JSON.stringify({ ok: false, error: 'no agent instrumentation' });
  }
  var raw = agent.snapshotCollect();
  try { window.__buzzSnapshotLast = raw; } catch (e) {}
  try {
    document.cookie = '__buzz_ba_snapshot=' + encodeURIComponent(raw) + '; path=/; SameSite=Lax';
  } catch (e2) {}
  return raw;
})();"#
    .to_string()
}

/// Legacy cookie-only path (kept for callers that only eval fire-and-forget).
pub fn snapshot_to_cookie_js() -> String {
    snapshot_collect_js()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_polls_after_cursor() {
        let buf = BrowserObserveBuffer::default();
        buf.push("playground-a", "console", None, 1);
        buf.push("playground-a", "nav", None, 2);
        let first = buf.poll("playground-a", 0, 10);
        assert_eq!(first.len(), 2);
        let after = buf.poll("playground-a", first[0].id, 10);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].kind, "nav");
    }

    #[test]
    fn push_nav_dedupes_same_url_and_fills_title() {
        let buf = BrowserObserveBuffer::default();
        assert!(buf.push_nav("playground-a", "https://a.test/", None, 1));
        assert!(!buf.push_nav("playground-a", "https://a.test/", None, 2));
        assert!(buf.push_nav("playground-a", "https://a.test/", Some("Hello"), 3));
        assert!(!buf.push_nav("playground-a", "https://a.test/", Some("Hello"), 4));
        assert!(!buf.push_nav("playground-a", "about:blank", Some("x"), 5));
        let events = buf.poll("playground-a", 0, 10);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].payload.as_ref().unwrap()["title"], "Hello");
    }

    #[test]
    fn instrumentation_mentions_network_and_cursor() {
        let js = instrumentation_js("playground-x", true);
        assert!(js.contains("fetch"));
        assert!(js.contains("__buzz_agent_cursor"));
        assert!(js.contains("setDrive"));
        assert!(js.contains("pushNavIfChanged"));
        assert!(js.contains("history[type]"));
        assert!(js.contains("requestNewTab"));
        assert!(js.contains("window.open"));
        assert!(js.contains("target=_blank"));
        assert!(js.contains("setNativeValue"));
        assert!(js.contains("enqueueAction"));
    }

    #[test]
    fn click_hit_test_skips_lock_and_cursor() {
        let js = instrumentation_js("playground-x", true);
        assert!(js.contains("elementsFromPoint"));
        assert!(js.contains("__buzz_agent_lock"));
        assert!(js.contains("isAgentChrome"));
        assert!(js.contains("hitTestGeometry"));
        assert!(js.contains("pointerEvents"));
        assert!(js.contains("pointerdown"));
        assert!(js.contains("pressKey"));
        assert!(js.contains("waitForCheck"));
    }

    #[test]
    fn drive_lock_blocks_trusted_human_scroll_events() {
        let js = instrumentation_js("playground-x", true);
        assert!(js.contains("blockHumanScroll"));
        assert!(js.contains("blockHumanScrollKey"));
        assert!(js.contains("addEventListener('wheel'"));
        assert!(js.contains("addEventListener('touchmove'"));
        assert!(js.contains("isTrusted === false"));
        assert!(js.contains("PageDown"));
    }

    #[test]
    fn user_event_instrumentation_emits_hit_and_value_length_not_value() {
        let js = instrumentation_js("playground-x", true);
        assert!(js.contains("push('user_click'"));
        assert!(js.contains("push('user_input'"));
        assert!(js.contains("push('user_keydown'"));
        assert!(js.contains("addEventListener('click', onUserClick"));
        assert!(js.contains("addEventListener('input', onUserInput"));
        assert!(js.contains("addEventListener('keydown', onUserKeydown"));
        assert!(js.contains("valueLength"));
        assert!(js.contains("describeUserHit"));
        assert!(js.contains("Never use el.value as name"));
        assert!(js.contains("Never attach a `value` field"));
        // userEventPayload must not assign payload.value / extra value leaking
        assert!(
            !js.contains("payload.value =") && !js.contains("payload['value']"),
            "user event payload must never set value"
        );
        // Printable key characters must not be emitted (PHI)
        assert!(js.contains("never typed characters") || js.contains("non-printable"));
    }

    #[test]
    fn drain_for_host_is_size_capped_and_resets_cursor() {
        let js = instrumentation_js("playground-x", false);
        assert!(js.contains("drainForHost"));
        assert!(js.contains("maxChars"));
        assert!(js.contains("afterId > seq"));
        let host_js = drain_page_queue_js(0, 25);
        assert!(host_js.contains("drainForHost"));
        assert!(!host_js.contains("__buzz_ba_drain"));
    }

    #[test]
    fn ingest_page_events_uses_separate_page_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let buf = BrowserObserveBuffer::default();
        buf.set_root(dir.path().to_path_buf());
        assert_eq!(buf.page_after_id("playground-a"), 0);
        let events = vec![
            PageDrainEvent {
                id: 1,
                kind: "console".into(),
                at_ms: Some(10),
                payload: Some(serde_json::json!({"level":"log","args":["hi"]})),
            },
            PageDrainEvent {
                id: 2,
                kind: "network".into(),
                at_ms: Some(11),
                payload: Some(serde_json::json!({
                    "phase":"response","url":"https://example.test/x",
                    "status":200,"body":"x".repeat(100)
                })),
            },
        ];
        assert_eq!(buf.ingest_page_events("playground-a", 0, &events, 99), 2);
        assert_eq!(buf.page_after_id("playground-a"), 2);
        // Stale expected_after must not double-ingest.
        assert_eq!(buf.ingest_page_events("playground-a", 0, &events, 99), 0);
        let polled = buf.poll("playground-a", 0, 10);
        assert_eq!(polled.len(), 2);
        assert_eq!(polled[0].kind, "console");
        assert_eq!(polled[1].kind, "network");
        // Host after_id is independent of page cursor.
        assert_eq!(buf.poll("playground-a", polled[0].id, 10).len(), 1);
        let jsonl =
            std::fs::read_to_string(dir.path().join("playground-a").join("events.jsonl")).unwrap();
        assert!(jsonl.contains("console"));
        assert!(jsonl.contains("network"));
    }

    #[test]
    fn is_new_tab_link_target_rules() {
        assert!(!is_new_tab_link_target(""));
        assert!(!is_new_tab_link_target("_self"));
        assert!(!is_new_tab_link_target("_parent"));
        assert!(!is_new_tab_link_target("_top"));
        assert!(is_new_tab_link_target("_blank"));
        assert!(is_new_tab_link_target("_new"));
        assert!(is_new_tab_link_target("popup"));
    }

    #[test]
    fn resolve_new_tab_url_https_only() {
        assert_eq!(
            resolve_new_tab_url("/x", "https://ex.test/a"),
            Some("https://ex.test/x".into())
        );
        assert_eq!(resolve_new_tab_url("http://ex.test/a", "https://ex.test/"), None);
        assert_eq!(resolve_new_tab_url("about:blank", "https://ex.test/"), None);
    }
}


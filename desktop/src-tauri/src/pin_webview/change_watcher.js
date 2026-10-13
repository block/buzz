// Buzz pinned-site change watcher. Injected into pin webviews only (main
// frame). It has no access to app APIs and sends NO page content: the only
// signal is a momentary invisible marker appended to document.title, which
// the native title-changed hook turns into a binary "changed" for this pin's
// label. Throttled so constantly ticking pages (live prices) ping at most
// once per THROTTLE_MS; the sidebar dot is binary anyway.
(function () {
  "use strict";
  if (window.top !== window || window.__buzzPinWatch) return;
  window.__buzzPinWatch = true;
  var MARK = "\u2063";
  var SETTLE_MS = 5000; // ignore initial load / hydration churn
  var DEBOUNCE_MS = 1500;
  var THROTTLE_MS = 20000;
  var start = Date.now();
  var lastPing = 0;
  var timer = 0;
  var pinging = false;

  function ping() {
    timer = 0;
    var now = Date.now();
    if (now - lastPing < THROTTLE_MS) {
      timer = setTimeout(ping, THROTTLE_MS - (now - lastPing));
      return;
    }
    lastPing = now;
    var title = document.title || "";
    if (title.indexOf(MARK) !== -1) return;
    pinging = true;
    document.title = title + MARK;
    setTimeout(function () {
      if (document.title === title + MARK) document.title = title;
      pinging = false;
    }, 60);
  }

  function schedule() {
    if (Date.now() - start < SETTLE_MS || timer) return;
    timer = setTimeout(ping, DEBOUNCE_MS);
  }

  function inTitle(node) {
    for (var n = node; n; n = n.parentNode) {
      if (n.nodeName === "TITLE") return true;
    }
    return false;
  }

  function hasText(nodes) {
    for (var i = 0; i < nodes.length; i++) {
      var n = nodes[i];
      if (n.nodeType === 3 && n.nodeValue && n.nodeValue.trim()) return true;
      if (n.nodeType === 1 && n.nodeName !== "SCRIPT" && n.nodeName !== "STYLE" &&
          n.textContent && n.textContent.trim()) return true;
    }
    return false;
  }

  function onMutations(list) {
    if (pinging) return;
    for (var i = 0; i < list.length; i++) {
      var m = list[i];
      if (inTitle(m.target)) continue; // title changes use the native hook
      if (m.type === "attributes") {
        // Only favicon swaps count; style/class churn never does.
        var rel = (m.target.getAttribute && m.target.getAttribute("rel")) || "";
        if (m.target.nodeName === "LINK" && /icon/i.test(rel)) return schedule();
        continue;
      }
      if (m.type === "characterData") {
        if ((m.target.nodeValue || "").trim() !== (m.oldValue || "").trim()) return schedule();
        continue;
      }
      if (hasText(m.addedNodes) || hasText(m.removedNodes)) return schedule();
    }
  }

  function begin() {
    try {
      new MutationObserver(onMutations).observe(document.documentElement, {
        childList: true,
        subtree: true,
        characterData: true,
        characterDataOldValue: true,
        attributes: true,
        attributeFilter: ["href"],
      });
    } catch (_) {}
  }
  if (document.documentElement) begin();
  else document.addEventListener("DOMContentLoaded", begin, { once: true });
})();

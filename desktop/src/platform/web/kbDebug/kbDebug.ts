// Web diagnostics overlay for keyboard / viewport layout and Buzz Term latency.
// On: open the app with ?kbdebug=1 (or Settings > Appearance > Diagnostics overlay); saved on this device.
// Off: ?kbdebug=0, the toggle, or the overlay's Off button. Vanilla DOM, outside the React tree; pointer-events only on
// its buttons, so it never takes taps from the app.
import { termProbe } from "./termProbe";

export const KBDEBUG_KEY = "buzz.kbdebug";
const listeners = new Set<() => void>();
let el: HTMLDivElement | null = null;
let body: HTMLPreElement | null = null;
let probe: HTMLDivElement | null = null;
let timer: number | undefined;
let pingTimer: number | undefined;
const log: string[] = [];
const t0 = performance.now();

export function isKbDebugOn(): boolean {
  try {
    return globalThis.localStorage?.getItem(KBDEBUG_KEY) === "1";
  } catch {
    return false;
  }
}
export function setKbDebug(on: boolean): void {
  try {
    if (on) localStorage.setItem(KBDEBUG_KEY, "1");
    else localStorage.removeItem(KBDEBUG_KEY);
  } catch {
    /* private mode */
  }
  if (on) show();
  else hide();
  for (const f of listeners) f();
}
export function subscribeKbDebug(f: () => void): () => void {
  listeners.add(f);
  return () => void listeners.delete(f);
}
/** ?kbdebug=1 / ?kbdebug=0 in the query or in the hash route. */
export function kbDebugFromUrl(href: string): boolean | null {
  const m = href.match(/[?&]kbdebug=([01])\b/);
  return m ? m[1] === "1" : null;
}

const r = (n: number | undefined | null) =>
  n == null || Number.isNaN(n) ? "–" : String(Math.round(n * 10) / 10);
const rect = (sel: string) => {
  const e = document.querySelector(sel);
  if (!e) return null;
  const b = e.getBoundingClientRect();
  return b.width || b.height ? b : null;
};

function safeAreas(): string {
  if (!probe) return "";
  const cs = getComputedStyle(probe);
  return `t${r(parseFloat(cs.paddingTop))} b${r(parseFloat(cs.paddingBottom))} l${r(parseFloat(cs.paddingLeft))} r${r(parseFloat(cs.paddingRight))}`;
}

/** Free-form line in the overlay's event log (e.g. push diagnostics). No-op cost when the overlay is off. */
export function kbNote(msg: string) {
  const t = ((performance.now() - t0) / 1000).toFixed(2);
  log.push(`${t} ${msg}`.slice(0, 120));
  if (log.length > 14) log.shift();
  if (el) render();
}

function note(type: string) {
  const vv = window.visualViewport;
  const t = ((performance.now() - t0) / 1000).toFixed(2);
  log.push(
    `${t} ${type} vv${r(vv?.height)}@${r(vv?.offsetTop)} ih${innerHeight} sy${r(scrollY)}`,
  );
  if (log.length > 14) log.shift();
  if (el) render();
}

function render() {
  if (!body) return;
  const vv = window.visualViewport;
  const html = document.documentElement;
  const cs = getComputedStyle(html);
  const root = document.getElementById("root");
  const rr = root?.getBoundingClientRect();
  const rcs = root ? getComputedStyle(root) : null;
  const comp = rect('[data-testid="message-composer"]');
  const dock = document.querySelector(".composer-dock");
  const tab = document.querySelector(
    '[data-testid="phone-tabbar"]',
  ) as HTMLElement | null;
  const nav = navigator as Navigator & { standalone?: boolean };
  const p = termProbe.snapshot();
  const s = (x: { n: number; avg: number; p95: number; last: number }) =>
    x.n ? `avg ${r(x.avg)} p95 ${r(x.p95)} last ${r(x.last)} (n${x.n})` : "–";
  const active = document.activeElement as HTMLElement | null;
  body.textContent = [
    `inner ${innerWidth}x${innerHeight} outer ${outerWidth}x${outerHeight} screen ${screen.width}x${screen.height}`,
    `vv h${r(vv?.height)} top${r(vv?.offsetTop)} pageTop${r(vv?.pageTop)} scale${r(vv?.scale)}  docClientH ${html.clientHeight} scrollY ${r(scrollY)}`,
    `safe-area ${safeAreas()}  standalone ${nav.standalone === true || matchMedia("(display-mode: standalone)").matches}`,
    `frame top${r(rr?.top)} h${r(rr?.height)} bottom${r(rr?.bottom)} pad t${rcs?.paddingTop} b${rcs?.paddingBottom}`,
    `--phone-vh ${cs.getPropertyValue("--phone-vh").trim() || "–"} --phone-vv-top ${cs.getPropertyValue("--phone-vv-top").trim() || "–"} kbd ${html.hasAttribute("data-phone-keyboard")}`,
    `composer bottom ${r(comp?.bottom)} (gap to vv bottom ${comp && vv ? r(vv.offsetTop + vv.height - comp.bottom) : "–"}) dock pb ${dock ? getComputedStyle(dock).paddingBottom : "–"}`,
    `tabbar ${tab ? (getComputedStyle(tab).display === "none" ? "hidden" : `shown top${r(tab.getBoundingClientRect().top)}`) : "none"}  focus ${active?.getAttribute("data-testid") || active?.tagName || "–"}`,
    `term ${p.transport || "no Direct connection"}`,
    `  key→echo ${s(p.echo)}`,
    `  key→paint ${s(p.paint)}`,
    `  ping ${s(p.ping)}`,
    "events:",
    ...log,
  ].join("\n");
}

function show() {
  if (el || typeof document === "undefined") return;
  termProbe.setEnabled(true);
  probe = document.createElement("div");
  probe.style.cssText =
    "position:fixed;left:0;top:0;width:0;height:0;visibility:hidden;pointer-events:none;padding:env(safe-area-inset-top) env(safe-area-inset-right) env(safe-area-inset-bottom) env(safe-area-inset-left)";
  el = document.createElement("div");
  el.setAttribute("data-testid", "kbdebug-overlay");
  el.style.cssText =
    "position:fixed;left:4px;right:4px;top:calc(env(safe-area-inset-top) + 2px);z-index:2147483646;pointer-events:none;background:rgba(0,0,0,.78);color:#9f9;border-radius:8px;padding:4px 6px;font:10px/1.25 ui-monospace,Menlo,monospace";
  const bar = document.createElement("div");
  bar.style.cssText =
    "display:flex;gap:6px;justify-content:flex-end;margin-bottom:2px";
  const btn = (label: string, fn: () => void, id: string) => {
    const b = document.createElement("button");
    b.type = "button";
    b.textContent = label;
    b.setAttribute("data-testid", id);
    b.style.cssText =
      "pointer-events:auto;font:inherit;color:#fff;background:#333;border:1px solid #666;border-radius:4px;padding:1px 6px";
    // don't steal focus from the composer / terminal while the keyboard is up
    b.addEventListener("pointerdown", (e) => e.preventDefault());
    b.addEventListener("click", fn);
    bar.appendChild(b);
  };
  btn(
    "Reset",
    () => {
      log.length = 0;
      termProbe.reset();
      render();
    },
    "kbdebug-reset",
  );
  btn(
    "Top/Bottom",
    () => {
      if (!el) return;
      const top = el.style.top !== "auto";
      el.style.top = top ? "auto" : "calc(env(safe-area-inset-top) + 2px)";
      el.style.bottom = top ? "4px" : "auto";
    },
    "kbdebug-move",
  );
  btn("Off", () => setKbDebug(false), "kbdebug-off");
  body = document.createElement("pre");
  body.style.cssText = "margin:0;white-space:pre-wrap;word-break:break-all";
  el.append(bar, body);
  document.documentElement.append(probe, el);
  render();
  timer = window.setInterval(render, 250);
  pingTimer = window.setInterval(
    () => void termProbe.pingOnce().then(render),
    2000,
  );
}
function hide() {
  window.clearInterval(timer);
  window.clearInterval(pingTimer);
  el?.remove();
  probe?.remove();
  el = body = probe = null;
  termProbe.setEnabled(false);
}

export function installKbDebug(): void {
  if (typeof window === "undefined") return;
  const fromUrl = kbDebugFromUrl(location.href);
  if (fromUrl !== null) setKbDebug(fromUrl);
  else if (isKbDebugOn()) show();
  window.addEventListener("hashchange", () => {
    const v = kbDebugFromUrl(location.href);
    if (v !== null && v !== isKbDebugOn()) setKbDebug(v);
  });
  const vv = window.visualViewport;
  vv?.addEventListener("resize", () => note("vv.resize"));
  vv?.addEventListener("scroll", () => note("vv.scroll"));
  window.addEventListener("resize", () => note("win.resize"));
  window.addEventListener("scroll", () => note("win.scroll"), {
    passive: true,
  });
  document.addEventListener(
    "focusin",
    (e) =>
      note(
        `focusin ${(e.target as HTMLElement)?.getAttribute?.("data-testid") || (e.target as HTMLElement)?.tagName}`,
      ),
    true,
  );
  document.addEventListener("focusout", () => note("focusout"), true);
  window.addEventListener("orientationchange", () => note("orientation"));
}

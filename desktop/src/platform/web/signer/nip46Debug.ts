// Console logging ([nip46] prefix, never secrets) + a tiny state object for the `?debug=1` on-screen line.
type Dbg = { relays: Record<string, boolean>; lastEvent: string | null; note: string | null };
const st: Dbg = { relays: {}, lastEvent: null, note: null };
const subs = new Set<() => void>();
const emit = () => subs.forEach((f) => f());
export const short = (h: string) => (h ? `${h.slice(0, 8)}…` : "?");
export const dbgEnabled = () => {
  try {
    return new URLSearchParams(location.search).get("debug") === "1";
  } catch {
    return false;
  }
};
export function nlog(msg: string, extra?: Record<string, unknown>) {
  if (extra) console.info("[nip46]", msg, extra);
  else console.info("[nip46]", msg);
  st.note = msg;
  emit();
}
export const nip46Debug = {
  get: () => st,
  setRelay(url: string, open: boolean) {
    st.relays[url] = open;
    emit();
  },
  forgetRelays() {
    st.relays = {};
    emit();
  },
  event(text: string) {
    st.lastEvent = `${new Date().toLocaleTimeString()} ${text}`;
    emit();
  },
  subscribe(f: () => void) {
    subs.add(f);
    return () => void subs.delete(f);
  },
};

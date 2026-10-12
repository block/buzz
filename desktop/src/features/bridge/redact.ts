// Redaction for anything the desktop publishes to the bridge (protocol doc §5/§9): secrets, env assignments and local paths.
// Conservative by design: a false positive only hides a few characters of a transcript.

const SECRET_PATTERNS: RegExp[] = [
  /\bnsec1[0-9a-z]{20,}\b/gi,
  /\bncryptsec1[0-9a-z]{20,}\b/gi,
  /\bnpub1[0-9a-z]{50,}\b(?=\s*[:=]\s*secret)/gi,
  /-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY-----|$)/g,
  /\bBearer\s+[A-Za-z0-9._~+/=-]{12,}/g,
  /\bBasic\s+[A-Za-z0-9+/=]{16,}/g,
  /\bsk-[A-Za-z0-9_-]{16,}/g,
  /\bgh[pousr]_[A-Za-z0-9]{20,}/g,
  /\bgithub_pat_[A-Za-z0-9_]{20,}/g,
  /\bxox[abpr]-[A-Za-z0-9-]{10,}/g,
  /\bAKIA[0-9A-Z]{16}\b/g,
  /\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}/g,
  /\b(?:AIza)[0-9A-Za-z_-]{30,}/g,
];

// KEY=value / KEY: value style env or config assignments with UPPER_SNAKE names.
const ENV_ASSIGN = /\b([A-Z][A-Z0-9_]{2,})\s*=\s*("[^"\n]*"|'[^'\n]*'|[^\s"']+)/g;
// JSON-ish "password": "x" / "api_key": "x"
const JSON_SECRET =
  /("(?:[A-Za-z0-9_-]*(?:key|token|secret|password|passwd|credential|authorization)[A-Za-z0-9_-]*)"\s*:\s*)"[^"\n]*"/gi;
const URL_CREDS = /\b([a-z][a-z0-9+.-]*:\/\/)[^\s/@:]+:[^\s/@]+@/gi;

// Absolute paths: home dirs collapse to "~", other deep absolute paths keep only the last two segments.
const HOME_PATH =
  /(?:\/Users|\/home)\/[^/\s"'`]+(?:\/[^\s"'`]*)?|[A-Za-z]:\\Users\\[^\s"'`]*/g;
const ABS_PATH = /(?<![\w.:~-])(\/(?:[\w.@+-]+\/){2,}[\w.@+-]+)/g;

export function redactText(input: string): string {
  let s = input;
  for (const re of SECRET_PATTERNS) s = s.replace(re, "[redacted]");
  s = s.replace(URL_CREDS, "$1[redacted]@");
  s = s.replace(JSON_SECRET, '$1"[redacted]"');
  s = s.replace(ENV_ASSIGN, (_m, k: string) => `${k}=[redacted]`);
  s = s.replace(HOME_PATH, (m) => {
    const parts = m.split(/[\\/]/).filter(Boolean).slice(m.startsWith("/") ? 2 : 3);
    return parts.length > 2 ? `~/…/${parts.slice(-2).join("/")}` : `~/${parts.join("/")}`;
  });
  s = s.replace(ABS_PATH, (_m, p: string) => {
    const parts = p.split("/").filter(Boolean);
    return `…/${parts.slice(-2).join("/")}`;
  });
  return s;
}

export function clipText(s: string, n: number): string {
  return s.length > n ? `${s.slice(0, n - 1)}…` : s;
}

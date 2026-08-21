// @ts-check

/**
 * @param {number} remainingPercent
 * @returns {"healthy" | "warning" | "critical"}
 */
export function providerUsageTone(remainingPercent) {
  if (remainingPercent < 20) return "critical";
  if (remainingPercent <= 50) return "warning";
  return "healthy";
}

/**
 * @param {number | null | undefined} value
 * @returns {string}
 */
export function formatTokenCount(value) {
  if (typeof value !== "number" || !Number.isFinite(value)) return "—";
  return new Intl.NumberFormat(undefined, {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(value);
}

/**
 * @param {number | null | undefined} epochSeconds
 * @returns {string}
 */
export function formatUsageReset(epochSeconds) {
  if (typeof epochSeconds !== "number" || !Number.isFinite(epochSeconds)) {
    return "Reset unavailable";
  }
  return new Intl.DateTimeFormat(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(epochSeconds * 1000));
}

/**
 * @param {unknown} error
 * @returns {string}
 */
export function providerUsageErrorMessage(error) {
  const code = typeof error === "string" ? error : String(error ?? "");
  if (code.includes("codex_not_installed")) return "Codex is not installed";
  if (code.includes("codex_not_authenticated")) {
    return "Sign in with Codex to show usage";
  }
  if (code.includes("protocol_unsupported")) {
    return "Update Codex to show usage";
  }
  if (code.includes("response_too_large")) {
    return "Codex returned an unsafe response";
  }
  return "Usage temporarily unavailable";
}

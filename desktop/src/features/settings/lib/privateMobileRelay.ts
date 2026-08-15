export interface PrivateMobileRelayResult {
  value: string;
  error: string | null;
}

const INVALID_PRIVATE_MOBILE_RELAY =
  "Enter an HTTPS *.ts.net origin without credentials, a path, query, or fragment.";

export function normalizePrivateMobileRelay(
  raw: string,
): PrivateMobileRelayResult {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { value: "", error: null };
  }

  const candidate = /^[a-z][a-z\d+.-]*:/i.test(trimmed)
    ? trimmed
    : `https://${trimmed}`;

  try {
    const url = new URL(candidate);
    const valid =
      url.protocol === "https:" &&
      url.hostname.toLowerCase().endsWith(".ts.net") &&
      url.username === "" &&
      url.password === "" &&
      (url.pathname === "" || url.pathname === "/") &&
      url.search === "" &&
      url.hash === "";

    if (!valid) {
      return { value: "", error: INVALID_PRIVATE_MOBILE_RELAY };
    }

    url.pathname = "/";
    return { value: url.toString(), error: null };
  } catch {
    return { value: "", error: INVALID_PRIVATE_MOBILE_RELAY };
  }
}

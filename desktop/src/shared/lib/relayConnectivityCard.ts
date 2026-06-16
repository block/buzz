import {
  isRelayUnreachableError,
  relayErrorDetail,
} from "@/shared/lib/relayError";

export type RelayConnectivityCardVariant =
  | "connect-vpn"
  | "reconnect-relay"
  | "refresh-access";

export function isBlockRelayUrl(relayUrl: string | null | undefined) {
  if (!relayUrl) {
    return false;
  }

  try {
    const url = new URL(
      relayUrl.replace("ws://", "http://").replace("wss://", "https://"),
    );
    const host = url.hostname.toLowerCase();
    return (
      host === "block.xyz" ||
      host.endsWith(".block.xyz") ||
      host === "sqprod.co" ||
      host.endsWith(".sqprod.co") ||
      host === "squareup.com" ||
      host.endsWith(".squareup.com")
    );
  } catch {
    return false;
  }
}

function normalizedRelayErrorDetail(errorMessage: string | null | undefined) {
  if (!errorMessage) {
    return "";
  }

  return (
    isRelayUnreachableError(errorMessage)
      ? relayErrorDetail(errorMessage)
      : errorMessage
  ).toLowerCase();
}

function isBlockConnectivityFailure(errorMessage: string | null | undefined) {
  if (!errorMessage) {
    return false;
  }

  if (isRelayUnreachableError(errorMessage)) {
    return true;
  }

  const detail = normalizedRelayErrorDetail(errorMessage);
  return (
    detail.includes("cloudflare access") ||
    detail.includes("network sign-in") ||
    detail.includes("sign-in required") ||
    detail.includes("vpn") ||
    detail.includes("proxy sign-in") ||
    detail.includes("http error: 302") ||
    detail.includes("302 found")
  );
}

function shouldRefreshBlockVpnAccess(errorMessage: string | null | undefined) {
  const detail = normalizedRelayErrorDetail(errorMessage);
  return (
    isBlockConnectivityFailure(errorMessage) &&
    (detail.includes("expired") ||
      detail.includes("unauthorized") ||
      detail.includes("forbidden") ||
      detail.includes("401") ||
      detail.includes("403"))
  );
}

export function resolveRelayConnectivityCardVariant(
  errorMessage: string | null | undefined,
  relayUrl: string | null | undefined,
): RelayConnectivityCardVariant {
  if (!isBlockRelayUrl(relayUrl)) {
    return "reconnect-relay";
  }

  if (shouldRefreshBlockVpnAccess(errorMessage)) {
    return "refresh-access";
  }

  return isBlockConnectivityFailure(errorMessage)
    ? "connect-vpn"
    : "reconnect-relay";
}

import { relayErrorDetail } from "@/shared/lib/relayError";

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

function shouldRefreshBlockVpnAccess(errorMessage: string | null | undefined) {
  if (!errorMessage) {
    return false;
  }

  const detail = relayErrorDetail(errorMessage).toLowerCase();
  return (
    detail.includes("re-authenticate") ||
    detail.includes("reauth") ||
    detail.includes("expired") ||
    detail.includes("unauthorized") ||
    detail.includes("forbidden") ||
    detail.includes("401") ||
    detail.includes("403")
  );
}

export function resolveRelayConnectivityCardVariant(
  errorMessage: string | null | undefined,
  relayUrl: string | null | undefined,
): RelayConnectivityCardVariant {
  if (!isBlockRelayUrl(relayUrl)) {
    return "reconnect-relay";
  }

  return shouldRefreshBlockVpnAccess(errorMessage)
    ? "refresh-access"
    : "connect-vpn";
}

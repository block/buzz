const MAX_CALLBACK_URL_LENGTH = 4_096;
const CALLBACK_FRAGMENT_KEY = "buzz_bind";
const CALLBACK_PAYLOAD_VERSION = "v1";

function encodeBase64Url(value: string): string {
  const bytes = new TextEncoder().encode(value);
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary)
    .replaceAll("+", "-")
    .replaceAll("/", "_")
    .replace(/=+$/, "");
}

export function buildNostrBindCallbackUrl(
  callbackUrl: string,
  signedResponse: string,
): string {
  const url = new URL(callbackUrl);
  url.hash = `${CALLBACK_FRAGMENT_KEY}=${CALLBACK_PAYLOAD_VERSION}.${encodeBase64Url(signedResponse)}`;
  const result = url.toString();
  if (result.length > MAX_CALLBACK_URL_LENGTH) {
    throw new Error("Signed response is too large to return to the browser.");
  }
  return result;
}

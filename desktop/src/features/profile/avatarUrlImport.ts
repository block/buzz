import { uploadMediaBytes, type BlobDescriptor } from "@/shared/api/tauri";
import { isRelayHostedAvatarUrl } from "@/shared/lib/avatarUrl";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";

const MAX_IMPORTED_AVATAR_BYTES = 10 * 1024 * 1024;
const IMPORT_TIMEOUT_MS = 15_000;
const ACCEPTED_IMPORTED_AVATAR_TYPES = new Set([
  "image/gif",
  "image/jpeg",
  "image/png",
  "image/webp",
]);

type UploadFn = (data: number[], filename?: string) => Promise<BlobDescriptor>;

type FetchFn = typeof fetch;

export type ImportAvatarUrlDependencies = {
  fetchFn?: FetchFn;
  uploadFn?: UploadFn;
  relayOrigin: string | null;
};

export async function importAvatarUrl(
  rawUrl: string,
  {
    fetchFn = fetch,
    relayOrigin,
    uploadFn = uploadMediaBytes,
  }: ImportAvatarUrlDependencies,
): Promise<BlobDescriptor> {
  const sourceUrl = parseImportUrl(rawUrl);
  const fetchUrl = isRelayHostedAvatarUrl(sourceUrl, relayOrigin)
    ? rewriteRelayUrl(sourceUrl)
    : sourceUrl;
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), IMPORT_TIMEOUT_MS);
  try {
    const response = await fetchFn(fetchUrl, {
      credentials: "omit",
      referrerPolicy: "no-referrer",
      signal: controller.signal,
    });
    if (!response.ok) {
      throw new Error(`Could not fetch avatar image (${response.status}).`);
    }
    assertImportImageHeaders(response.headers);
    const blob = await response.blob();
    assertImportImageBlob(blob);
    const buffer = await blob.arrayBuffer();
    const uploaded = await uploadFn(
      [...new Uint8Array(buffer)],
      filenameForImportedAvatar(sourceUrl, blob.type),
    );
    if (!isAcceptedAvatarImageType(uploaded.type)) {
      throw new Error("Choose a PNG, JPG, GIF, or WebP image.");
    }
    return uploaded;
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      throw new Error("Avatar image fetch timed out.");
    }
    throw error;
  } finally {
    clearTimeout(timeout);
  }
}

export function parseImportUrl(rawUrl: string): string {
  const trimmed = rawUrl.trim();
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    throw new Error("Enter a valid HTTP(S) image URL.");
  }
  if (parsed.protocol !== "https:" && parsed.protocol !== "http:") {
    throw new Error("Enter a valid HTTP(S) image URL.");
  }
  return parsed.toString();
}

function assertImportImageHeaders(headers: Headers): void {
  const contentType = headers.get("content-type")?.toLowerCase() ?? "";
  if (contentType && !isAcceptedAvatarImageType(contentType)) {
    throw new Error("Choose a PNG, JPG, GIF, or WebP image.");
  }
  const contentLength = Number(headers.get("content-length") ?? "0");
  if (contentLength > MAX_IMPORTED_AVATAR_BYTES) {
    throw new Error("Avatar image is too large.");
  }
}

function assertImportImageBlob(blob: Blob): void {
  if (blob.size === 0) {
    throw new Error("Avatar image is empty.");
  }
  if (blob.size > MAX_IMPORTED_AVATAR_BYTES) {
    throw new Error("Avatar image is too large.");
  }
  if (blob.type && !isAcceptedAvatarImageType(blob.type)) {
    throw new Error("Choose a PNG, JPG, GIF, or WebP image.");
  }
}

function isAcceptedAvatarImageType(contentType: string): boolean {
  return ACCEPTED_IMPORTED_AVATAR_TYPES.has(
    contentType.toLowerCase().split(";")[0]?.trim() ?? "",
  );
}

function filenameForImportedAvatar(
  sourceUrl: string,
  contentType: string,
): string {
  const pathSegment = new URL(sourceUrl).pathname.split("/").pop() ?? "";
  const cleanName = pathSegment.replace(/[^\w.-]/gu, "_");
  if (/\.(?:png|jpe?g|gif|webp)$/iu.test(cleanName)) return cleanName;
  return `avatar.${extensionForImageType(contentType)}`;
}

function extensionForImageType(contentType: string): string {
  switch (contentType.toLowerCase()) {
    case "image/gif":
      return "gif";
    case "image/jpeg":
      return "jpg";
    case "image/webp":
      return "webp";
    default:
      return "png";
  }
}

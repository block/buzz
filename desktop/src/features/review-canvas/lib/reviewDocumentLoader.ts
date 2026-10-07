import { ReviewContractError, type ReviewRevision } from "./reviewContract";
import { parseReviewDocument, type ReviewDocument } from "./reviewDocument";

export async function sha256Hex(
  bytes: Uint8Array<ArrayBuffer>,
): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

/**
 * Fetch, verify, and parse the exact document a review revision names.
 *
 * The native layer already refuses mismatched bytes; this re-checks the size
 * and SHA-256 in the webview so nothing is parsed, let alone rendered, unless
 * the bytes it holds are the bytes the signed revision promised.
 */
export async function loadReviewDocument(
  revision: Pick<ReviewRevision, "document">,
  fetchBytes: (input: {
    url: string;
    expectedSha256: string;
    expectedSize: number;
  }) => Promise<Uint8Array<ArrayBuffer>>,
): Promise<ReviewDocument> {
  const { url, sha256, size } = revision.document;
  const bytes = await fetchBytes({
    url,
    expectedSha256: sha256,
    expectedSize: size,
  });
  if (bytes.byteLength !== size) {
    throw new ReviewContractError(
      "The review document size does not match the review revision.",
    );
  }
  if ((await sha256Hex(bytes)) !== sha256) {
    throw new ReviewContractError(
      "The review document does not match its recorded SHA-256, so it was not rendered.",
    );
  }
  let html: string;
  try {
    html = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    throw new ReviewContractError("The review document is not valid UTF-8.");
  }
  return parseReviewDocument(html);
}

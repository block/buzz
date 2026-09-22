/**
 * Pure classification + submit gating for the key-import form, unit-testable
 * without a DOM.
 *
 * `ncryptsec1…` is a NIP-49 encrypted backup: no npub preview is possible
 * (the pubkey is inside the encrypted payload) and a passphrase is required.
 * Password validation happens in Rust at decrypt time; this module performs
 * the password-independent Bech32 and NIP-49 structure checks needed to decide
 * when the form can safely switch modes.
 */

import { nsecToNpub } from "@/shared/lib/nostrUtils";

export type KeyImportKind = "nsec" | "ncryptsec" | "unknown";

const NCRYPTSEC_HRP = "ncryptsec";
const NIP49_VERSION = 2;
const NIP49_PAYLOAD_BYTES = 91;
/** Current NIP-49 payloads encode to 162 characters including the checksum. */
export const NCRYPTSEC_ENCODED_LENGTH = 162;
const BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const BECH32_GENERATORS = [
  0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3,
] as const;

function bech32Polymod(values: readonly number[]): number {
  let checksum = 1;
  for (const value of values) {
    const high = checksum >>> 25;
    checksum = ((checksum & 0x1ffffff) << 5) ^ value;
    for (let index = 0; index < BECH32_GENERATORS.length; index += 1) {
      if ((high >>> index) & 1) checksum ^= BECH32_GENERATORS[index];
    }
  }
  return checksum >>> 0;
}

function expandBech32Hrp(hrp: string): number[] {
  return [
    ...Array.from(hrp, (character) => character.charCodeAt(0) >>> 5),
    0,
    ...Array.from(hrp, (character) => character.charCodeAt(0) & 31),
  ];
}

function convertFiveBitWordsToBytes(words: readonly number[]): number[] | null {
  let accumulator = 0;
  let bitCount = 0;
  const bytes: number[] = [];

  for (const word of words) {
    accumulator = (accumulator << 5) | word;
    bitCount += 5;
    while (bitCount >= 8) {
      bitCount -= 8;
      bytes.push((accumulator >>> bitCount) & 0xff);
    }
  }

  // Bech32 conversion without padding permits fewer than five zero remainder
  // bits. Any larger or non-zero remainder is not a canonical byte encoding.
  if (bitCount >= 5 || ((accumulator << (8 - bitCount)) & 0xff) !== 0) {
    return null;
  }
  return bytes;
}

export function classifyKeyImportInput(input: string): KeyImportKind {
  const trimmed = input.trim();
  // Case-insensitive on the HRP to match the Rust classifier: an uppercase
  // valid backup routes to the encrypted path (and decodes there); mixed
  // case routes there too and fails in Rust with the accurate error.
  if (trimmed.slice(0, 10).toLowerCase() === "ncryptsec1") return "ncryptsec";
  if (trimmed.startsWith("nsec1")) return "nsec";
  return "unknown";
}

/**
 * Password-independent NIP-49 validation used for the automatic UI transition.
 * A candidate must have canonical casing and length, a valid Bech32 checksum,
 * and the current 91-byte/version-2 NIP-49 payload shape.
 */
export function isPlausibleNcryptsec(input: string): boolean {
  const trimmed = input.trim();
  if (trimmed.length !== NCRYPTSEC_ENCODED_LENGTH) return false;
  if (trimmed !== trimmed.toLowerCase() && trimmed !== trimmed.toUpperCase()) {
    return false;
  }

  const normalized = trimmed.toLowerCase();
  const separatorIndex = normalized.lastIndexOf("1");
  if (
    separatorIndex !== NCRYPTSEC_HRP.length ||
    normalized.slice(0, separatorIndex) !== NCRYPTSEC_HRP
  ) {
    return false;
  }

  const encoded = normalized.slice(separatorIndex + 1);
  const words = Array.from(encoded, (character) =>
    BECH32_CHARSET.indexOf(character),
  );
  if (words.some((word) => word < 0) || words.length <= 6) return false;
  if (bech32Polymod([...expandBech32Hrp(NCRYPTSEC_HRP), ...words]) !== 1) {
    return false;
  }

  const payload = convertFiveBitWordsToBytes(words.slice(0, -6));
  return (
    payload?.length === NIP49_PAYLOAD_BYTES && payload[0] === NIP49_VERSION
  );
}

/**
 * Whether the import form's submit should be enabled.
 * nsec: must derive an npub. ncryptsec: plausible blob + non-empty passphrase.
 */
export function keyImportSubmitEnabled(
  input: string,
  passphrase: string,
): boolean {
  const kind = classifyKeyImportInput(input);
  if (kind === "ncryptsec") {
    return isPlausibleNcryptsec(input) && passphrase.length > 0;
  }
  return nsecToNpub(input) !== null;
}

/** PNG file signature (first 4 bytes). */
const PNG_MAGIC = [0x89, 0x50, 0x4e, 0x47] as const;

/** Opening brace `{` — first byte of a JSON file. */
const JSON_FIRST_BYTE = 0x7b;

function matchesMagic(
  bytes: number[] | readonly number[],
  magic: readonly number[],
): boolean {
  return magic.every((b, i) => bytes[i] === b);
}

function isMarkdownFileName(fileName: string | undefined): boolean {
  return fileName?.toLowerCase().endsWith(".md") ?? false;
}

/** Return true when the file format carries one persona instead of a bundle. */
export function isSingleItemFile(
  bytes: number[] | readonly number[],
  fileName?: string,
): boolean {
  return (
    matchesMagic(bytes, PNG_MAGIC) ||
    (bytes.length > 0 && bytes[0] === JSON_FIRST_BYTE) ||
    isMarkdownFileName(fileName)
  );
}

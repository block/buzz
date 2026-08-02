export function normalizeSuppressedLinkPreviewUrls(
  values: Iterable<string> | undefined,
): string[];
export function getLinkPreviewSuppression(
  tags: string[][] | readonly (readonly string[])[] | undefined,
): { all: boolean; urls: string[] };
export function buildLinkPreviewSuppressionTags(
  values: Iterable<string> | undefined,
): string[][];

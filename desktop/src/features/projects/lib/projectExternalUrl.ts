/** Builds a GitHub repository URL scoped to the selected branch or tag. */
export function projectExternalRefUrl(
  externalUrl: string | null | undefined,
  ref: string | null | undefined,
): string | null {
  if (!externalUrl) return null;
  const selectedRef = ref?.trim();
  if (!selectedRef) return externalUrl;

  try {
    const url = new URL(externalUrl);
    if (
      url.protocol !== "https:" ||
      url.hostname.toLowerCase() !== "github.com"
    ) {
      return externalUrl;
    }
    const segments = url.pathname.split("/").filter(Boolean);
    if (segments.length !== 2) return externalUrl;
    const repository = segments[1]?.replace(/\.git$/i, "");
    if (!segments[0] || !repository) return externalUrl;
    return `${url.origin}/${segments[0]}/${repository}/tree/${encodeURIComponent(selectedRef)}`;
  } catch {
    return externalUrl;
  }
}

const COMMIT_PATTERN = /^(?:[0-9a-fA-F]{40}|[0-9a-fA-F]{64})$/;
const BRANCH_CHARACTERS = /^[A-Za-z0-9/_.-]+$/;
const MAX_REPOSITORY_PATH_BYTES = 4096;
const UTF8_ENCODER = new TextEncoder();

export type RepositoryRefKind = "branch" | "commit";

export type ParsedRepositoryRef = {
  kind: RepositoryRefKind;
  value: string;
};

/** Parse a conservative branch name or a full 40/64-character commit hash. */
export function parseRepositoryRef(value: string): ParsedRepositoryRef | null {
  const trimmed = value.trim();
  if (COMMIT_PATTERN.test(trimmed)) {
    return { kind: "commit", value: trimmed.toLowerCase() };
  }
  if (trimmed.startsWith("refs/") && !trimmed.startsWith("refs/heads/")) {
    return null;
  }
  const branch = trimmed.replace(/^refs\/heads\//, "");
  if (
    !branch ||
    branch.startsWith("-") ||
    branch.startsWith("/") ||
    branch.endsWith("/") ||
    branch.endsWith(".") ||
    branch.endsWith(".lock") ||
    branch.includes("..") ||
    branch.includes("//") ||
    !BRANCH_CHARACTERS.test(branch) ||
    branch.split("/").some((component) => component.startsWith("."))
  ) {
    return null;
  }
  return { kind: "branch", value: branch };
}

function containsControlCharacter(value: string): boolean {
  return Array.from(value).some((character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    return codePoint <= 0x1f || codePoint === 0x7f;
  });
}

/** Validate a repository-relative file or directory coordinate. */
export function normalizeRepositoryPath(value: string): string | null {
  if (
    !value ||
    UTF8_ENCODER.encode(value).length > MAX_REPOSITORY_PATH_BYTES ||
    value.startsWith("/") ||
    value.startsWith("\\") ||
    value.endsWith("/") ||
    value.includes("\\") ||
    containsControlCharacter(value)
  ) {
    return null;
  }
  const segments = value.split("/");
  if (
    segments.some((segment) => !segment || segment === "." || segment === "..")
  ) {
    return null;
  }
  return value;
}

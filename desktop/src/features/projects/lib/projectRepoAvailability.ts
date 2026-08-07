export type ProjectRepoUnavailableReason =
  | "missing"
  | "authentication"
  | "network"
  | "ref"
  | "unknown";

export function projectRepoUnavailableReason(
  error: unknown,
): ProjectRepoUnavailableReason {
  const message =
    error instanceof Error
      ? error.message.toLowerCase()
      : typeof error === "string"
        ? error.toLowerCase()
        : "";

  if (!message) return "missing";
  if (
    /\b(?:401|403)\b|authenticat|authoriz|permission denied|access denied/.test(
      message,
    )
  ) {
    return "authentication";
  }
  if (
    /\b404\b|repository not found|repository does not exist|not found on the relay/.test(
      message,
    )
  ) {
    return "missing";
  }
  if (
    /remote branch .* not found|could not resolve the requested repository ref|couldn't find remote ref/.test(
      message,
    )
  ) {
    return "ref";
  }
  if (
    /timed? out|could not resolve host|failed to connect|connection (?:refused|reset)|network is unreachable|offline/.test(
      message,
    )
  ) {
    return "network";
  }
  return "unknown";
}

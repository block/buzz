export type ProjectRepoUnavailableReason =
  | "missing"
  | "access"
  | "unbound"
  | "authentication"
  | "network"
  | "ref"
  | "unknown";

/** User-facing copy for a classified repository availability failure. */
export type ProjectRepoUnavailablePresentation = {
  description: string;
  title: string;
};

const PROJECT_REPO_UNAVAILABLE_PRESENTATIONS: Record<
  ProjectRepoUnavailableReason,
  ProjectRepoUnavailablePresentation
> = {
  authentication: {
    description:
      "Buzz could not authenticate with this repository. Check your access and try again.",
    title: "Repository access failed",
  },
  missing: {
    description:
      "The project announcement exists, but its git repository was not found on the Buzz relay.",
    title: "Repository not initialized",
  },
  access: {
    description:
      "Repository access is granted through its channel, and you’re not a member. Ask the repository owner for an invite.",
    title: "Repository access restricted",
  },
  unbound: {
    description:
      "This repository has no access channel binding, so the relay cannot authorize anyone to read it. The repository owner can bind a channel from the Access menu.",
    title: "No access channel bound",
  },
  network: {
    description:
      "The Buzz git service could not be reached. Check your connection and try again.",
    title: "Couldn’t reach repository",
  },
  ref: {
    description:
      "The selected branch is advertised by the project but is missing from its git remote.",
    title: "Branch unavailable",
  },
  unknown: {
    description:
      "Buzz could not load this repository. Try again or contact the project owner.",
    title: "Repository unavailable",
  },
};

/** Returns consistent, sanitized copy for repository availability UI. */
export function projectRepoUnavailablePresentation(
  reason: ProjectRepoUnavailableReason,
): ProjectRepoUnavailablePresentation {
  return PROJECT_REPO_UNAVAILABLE_PRESENTATIONS[reason];
}

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

/**
 * The relay deliberately answers channel-ACL denials with the same 404 as a
 * genuinely absent repository (SEC-005 anti-enumeration), so the git error
 * alone cannot distinguish "never initialized" from "you have no access".
 * The announcement events ARE visible to every relay member though, so the
 * client can re-classify a `missing` result using the repository's
 * `buzz-channel` binding and the viewer's own channel memberships:
 *
 * - no binding at all → `unbound` (the relay refuses access for everyone
 *   until the owner binds a channel)
 * - bound to a channel the viewer is not a member of → `access`
 * - bound to a channel the viewer IS a member of → keep `missing` (the
 *   repository truly has no git data pointer on the relay)
 *
 * `memberChannelIds === null` means memberships are still loading — the
 * reason is left untouched rather than guessed.
 */
export function refineRepoUnavailableReason(input: {
  reason: ProjectRepoUnavailableReason;
  repositoryChannelId: string | null | undefined;
  memberChannelIds: readonly string[] | null;
}): ProjectRepoUnavailableReason {
  if (input.reason !== "missing") return input.reason;
  if (!input.repositoryChannelId) return "unbound";
  if (input.memberChannelIds === null) return input.reason;
  return input.memberChannelIds.includes(input.repositoryChannelId)
    ? input.reason
    : "access";
}

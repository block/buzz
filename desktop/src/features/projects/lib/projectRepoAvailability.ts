import { i18n } from "@/i18n";

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

/** Returns consistent, sanitized copy for repository availability UI. */
export function projectRepoUnavailablePresentation(
  reason: ProjectRepoUnavailableReason,
): ProjectRepoUnavailablePresentation {
  if (reason === "authentication") {
    return {
      description: i18n.t("projects.unavailable.authentication-description"),
      title: i18n.t("projects.unavailable.authentication-title"),
    };
  }
  if (reason === "missing") {
    return {
      description: i18n.t("projects.unavailable.missing-description"),
      title: i18n.t("projects.unavailable.missing-title"),
    };
  }
  if (reason === "access") {
    return {
      description: i18n.t("projects.unavailable.access-description"),
      title: i18n.t("projects.unavailable.access-title"),
    };
  }
  if (reason === "unbound") {
    return {
      description: i18n.t("projects.unavailable.unbound-description"),
      title: i18n.t("projects.unavailable.unbound-title"),
    };
  }
  if (reason === "network") {
    return {
      description: i18n.t("projects.unavailable.network-description"),
      title: i18n.t("projects.unavailable.network-title"),
    };
  }
  if (reason === "ref") {
    return {
      description: i18n.t("projects.unavailable.ref-description"),
      title: i18n.t("projects.unavailable.ref-title"),
    };
  }
  return {
    description: i18n.t("projects.unavailable.unknown-description"),
    title: i18n.t("projects.unavailable.unknown-title"),
  };
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

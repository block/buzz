import type { ProjectRepoUnavailableReason } from "./projectRepoAvailability";

export type ProjectGitErrorPresentation = {
  title: string;
  description: string;
};

function errorText(error: unknown) {
  if (error instanceof Error) return error.message.toLowerCase();
  return typeof error === "string" ? error.toLowerCase() : "";
}

function isGitHubUrl(cloneUrl: string | null | undefined) {
  try {
    return new URL(cloneUrl ?? "").hostname.toLowerCase() === "github.com";
  } catch {
    return false;
  }
}

/**
 * Recognises the relay's git route by the same path boundary `projectRepoHost`
 * enforces. The origin cannot be compared here — presentation has no relay
 * origin to hand — but a `/git/<owner-pubkey>/<repo>` path is only ever served
 * by a Buzz relay, and it is enough to pick the right recovery advice.
 */
function isBuzzRelayUrl(cloneUrl: string | null | undefined) {
  try {
    return /^\/git\/[0-9a-f]{64}\/[^/]+\/?$/i.test(
      new URL(cloneUrl ?? "").pathname,
    );
  } catch {
    return false;
  }
}

export function projectCloneErrorPresentation(
  error: unknown,
  cloneUrl?: string | null,
  unavailableReason?: ProjectRepoUnavailableReason,
): ProjectGitErrorPresentation {
  const message = errorText(error);
  const github = isGitHubUrl(cloneUrl);

  if (unavailableReason === "access") {
    return {
      title: "Repository access restricted",
      description:
        "You need access to the repository’s channel before you can clone it.",
    };
  }

  // Buzz runs git with `credential.helper` cleared, `GIT_CONFIG_GLOBAL`
  // pointed at /dev/null and `GIT_TERMINAL_PROMPT=0` (`project_git_exec.rs`)
  // — deliberately, because every process git spawns inherits an environment
  // holding NOSTR_PRIVATE_KEY. A private HTTPS remote therefore fails with
  // git's non-interactive credential error, which matches none of the auth
  // patterns below and lands on "try again" — advice that can never work.
  //
  // Only git's own `could not read Username/Password for <url>` signature is
  // matched. The trailing reason varies by platform (`terminal prompts
  // disabled` under GIT_TERMINAL_PROMPT=0, `No such device or address` on a
  // TTY-less Unix, `Device not configured` on macOS), so anchoring on the
  // prefix covers all three without claiming a credential failure for the
  // transport and filesystem errors those generic OS phrases also accompany.
  if (/could not read (?:username|password) for /.test(message)) {
    if (isBuzzRelayUrl(cloneUrl)) {
      return {
        title: "Relay wouldn’t authenticate this clone",
        description:
          "Check that the relay hosting this repository is connected and that your Buzz identity has access to it.",
      };
    }
    return {
      title: "Repository needs credentials Buzz can’t supply",
      description: github
        ? "Buzz clones with credential helpers disabled, so a private GitHub repository over HTTPS cannot authenticate. Clone it outside Buzz, push it to a Buzz-hosted repository, then clone from that repository’s Buzz URL."
        : "Buzz clones with credential helpers disabled, so this repository cannot authenticate over HTTPS. Clone it outside Buzz, push it to a Buzz-hosted repository, then clone from that repository’s Buzz URL.",
    };
  }
  if (
    /\b(?:401|403)\b|authenticat|authoriz|permission denied|access denied|ssh certificate/.test(
      message,
    )
  ) {
    return {
      title: "Repository access required",
      description: github
        ? "This repository requires GitHub authentication. Buzz currently clones public GitHub repositories without credentials."
        : "Buzz could not authenticate with this repository. Check your access and try again.",
    };
  }
  if (/\b404\b|repository not found|repository does not exist/.test(message)) {
    return {
      title: "Repository not found",
      description:
        "Check that the repository link is correct and that the repository still exists.",
    };
  }
  if (
    /timed? out|could not resolve host|failed to connect|connection (?:refused|reset)|network is unreachable|offline/.test(
      message,
    )
  ) {
    return {
      title: "Couldn’t reach the repository",
      description: "Check your connection and try cloning again.",
    };
  }
  if (
    /already exists and is not an empty directory|destination path .* exists/.test(
      message,
    )
  ) {
    return {
      title: "Local folder already exists",
      description:
        "Choose a different repositories directory or remove the existing checkout.",
    };
  }
  return {
    title: "Couldn’t clone repository",
    description: github
      ? "Try again, or open the repository on GitHub for more information."
      : "Try again. If the problem continues, contact the repository owner.",
  };
}

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

// Operator-trusted external origins beyond github.com (see
// `BUZZ_TRUSTED_EXTERNAL_GIT_ORIGINS` in project_git_exec.rs) are typically
// self-hosted GitLab instances, authenticated via `glab`. The frontend has
// no visibility into the exact trust list, so this is a hostname heuristic
// rather than an exact match.
function isGitLabUrl(cloneUrl: string | null | undefined) {
  try {
    return new URL(cloneUrl ?? "").hostname.toLowerCase().includes("gitlab");
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
  const gitlab = isGitLabUrl(cloneUrl);

  if (unavailableReason === "access") {
    return {
      title: "Repository access restricted",
      description:
        "You need access to the repository’s channel before you can clone it.",
    };
  }
  if (message.includes("must be github.com or a host listed in")) {
    return {
      title: "Repository host not trusted",
      description:
        "Ask your Buzz operator to add this GitLab origin to `BUZZ_TRUSTED_EXTERNAL_GIT_ORIGINS` and restart Buzz.",
    };
  }
  if (message.includes("install the gh cli")) {
    return {
      title: "GitHub CLI required",
      description:
        "Install the GitHub CLI, run `gh auth login`, restart Buzz, and try again.",
    };
  }
  if (message.includes("install the glab cli")) {
    return {
      title: "GitLab CLI required",
      description:
        "Install the GitLab CLI, run `glab auth login`, restart Buzz, and try again.",
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
        ? "This repository requires GitHub authentication. Sign in with the GitHub CLI and try again."
        : gitlab
          ? "This repository requires GitLab authentication. Sign in with the GitLab CLI (`glab auth login`) and try again."
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

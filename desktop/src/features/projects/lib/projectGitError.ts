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

export function projectCloneErrorPresentation(
  error: unknown,
  cloneUrl?: string | null,
): ProjectGitErrorPresentation {
  const message = errorText(error);
  const github = isGitHubUrl(cloneUrl);

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

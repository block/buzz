// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
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

export function projectCloneErrorPresentation(
  error: unknown,
  cloneUrl?: string | null,
  unavailableReason?: ProjectRepoUnavailableReason,
): ProjectGitErrorPresentation {
  const message = errorText(error);
  const github = isGitHubUrl(cloneUrl);

  if (unavailableReason === "access") {
    return {
      get title() {
        return uiT("Repository access restricted");
      },
      get description() {
        return uiT(
          "You need access to the repository’s channel before you can clone it.",
        );
      },
    };
  }
  if (
    /\b(?:401|403)\b|authenticat|authoriz|permission denied|access denied|ssh certificate/.test(
      message,
    )
  ) {
    return {
      get title() {
        return uiT("Repository access required");
      },
      description: github
        ? uiT(
            "This repository requires GitHub authentication. Buzz currently clones public GitHub repositories without credentials.",
          )
        : uiT(
            "Buzz could not authenticate with this repository. Check your access and try again.",
          ),
    };
  }
  if (/\b404\b|repository not found|repository does not exist/.test(message)) {
    return {
      get title() {
        return uiT("Repository not found");
      },
      get description() {
        return uiT(
          "Check that the repository link is correct and that the repository still exists.",
        );
      },
    };
  }
  if (
    /timed? out|could not resolve host|failed to connect|connection (?:refused|reset)|network is unreachable|offline/.test(
      message,
    )
  ) {
    return {
      get title() {
        return uiT("Couldn’t reach the repository");
      },
      get description() {
        return uiT("Check your connection and try cloning again.");
      },
    };
  }
  if (
    /already exists and is not an empty directory|destination path .* exists/.test(
      message,
    )
  ) {
    return {
      get title() {
        return uiT("Local folder already exists");
      },
      get description() {
        return uiT(
          "Choose a different repositories directory or remove the existing checkout.",
        );
      },
    };
  }
  return {
    get title() {
      return uiT("Couldn’t clone repository");
    },
    description: github
      ? uiT("Try again, or open the repository on GitHub for more information.")
      : uiT(
          "Try again. If the problem continues, contact the repository owner.",
        ),
  };
}

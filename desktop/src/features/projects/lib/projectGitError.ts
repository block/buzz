import { i18n } from "@/i18n";
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
      title: i18n.t("projects.unavailable.access-title"),
      description: i18n.t("projects.clone-error.access-restricted-description"),
    };
  }
  if (
    /\b(?:401|403)\b|authenticat|authoriz|permission denied|access denied|ssh certificate/.test(
      message,
    )
  ) {
    return {
      title: i18n.t("projects.clone-error.access-required-title"),
      description: github
        ? i18n.t("projects.clone-error.access-required-github")
        : i18n.t("projects.unavailable.authentication-description"),
    };
  }
  if (/\b404\b|repository not found|repository does not exist/.test(message)) {
    return {
      title: i18n.t("projects.clone-error.not-found-title"),
      description: i18n.t("projects.clone-error.not-found-description"),
    };
  }
  if (
    /timed? out|could not resolve host|failed to connect|connection (?:refused|reset)|network is unreachable|offline/.test(
      message,
    )
  ) {
    return {
      title: i18n.t("projects.clone-error.unreachable-title"),
      description: i18n.t("projects.clone-error.unreachable-description"),
    };
  }
  if (
    /already exists and is not an empty directory|destination path .* exists/.test(
      message,
    )
  ) {
    return {
      title: i18n.t("projects.clone-error.folder-exists-title"),
      description: i18n.t("projects.clone-error.folder-exists-description"),
    };
  }
  return {
    title: i18n.t("projects.clone-error.fallback-title"),
    description: github
      ? i18n.t("projects.clone-error.fallback-github")
      : i18n.t("projects.clone-error.fallback-description"),
  };
}

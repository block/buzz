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
  // Key this on git's own credential signatures only. The trailing reason git
  // prints for a missing tty ("No such device or address", "Device not
  // configured") is also what an unrelated filesystem or device failure
  // reports, and git always prefixes the credential case with "could not read
  // Username/Password", so matching the reason alone only overmatches.
  if (
    /could not read (?:username|password)|terminal prompts disabled/.test(
      message,
    )
  ) {
    return {
      title: "Repository needs credentials Buzz can’t supply",
      description: github
        ? "Buzz clones with credential helpers disabled, so a private GitHub repository over HTTPS cannot authenticate. Announce the repository on your Buzz relay to clone it through Buzz."
        : "Buzz clones with credential helpers disabled, so this repository cannot authenticate over HTTPS. Announce the repository on your Buzz relay to clone it through Buzz.",
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
  // `configure_git_auth` allows only http/https (and file, on request), so a
  // pasted `git@host:repo` URL never reaches the network: git refuses the
  // transport outright. Retrying cannot change that — the URL has to change.
  if (
    /transport '[^']+' not allowed|protocol '[^']+' is not supported/.test(
      message,
    )
  ) {
    return {
      title: "Clone URL uses an unsupported transport",
      description:
        "Buzz clones over HTTPS only. Use the repository’s HTTPS clone URL instead of an SSH or git:// one.",
    };
  }
  // A proxy or self-signed certificate in front of the host. "Try again"
  // never resolves it; the certificate chain has to be trusted first.
  //
  // Match concrete trust/verification signatures only. Every backend also
  // reports ordinary transport failures under its handshake prefix
  // (`gnutls_handshake`, `schannel:`), and telling someone to install a CA
  // when the certificate was never the problem sends them the wrong way;
  // those fall through to the connectivity message below.
  if (
    /ssl certificate problem|unable to get local issuer certificate|unable to verify the first certificate|certificate verify failed|error in the certificate verification|self[- ]signed certificate|certificate (?:chain )?(?:is |was )?not trusted|sec_e_untrusted_root|cert_e_untrustedroot/.test(
      message,
    )
  ) {
    return {
      title: "Couldn’t verify the server’s certificate",
      description:
        "The TLS certificate for this host could not be verified — often a corporate proxy or a self-signed certificate. Install the trusted CA certificate your administrator provides, then try again. Don’t turn off certificate verification.",
    };
  }
  if (/\b404\b|repository not found|repository does not exist/.test(message)) {
    return {
      title: "Repository not found",
      description:
        "Check that the repository link is correct and that the repository still exists.",
    };
  }
  // A handshake that failed for anything other than trust — a prematurely
  // terminated TLS connection, a dropped proxy — is a connectivity problem.
  if (
    /timed? out|could not resolve host|failed to connect|connection (?:refused|reset)|network is unreachable|offline|gnutls_handshake|schannel: |ssl connect error|handshake fail/.test(
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

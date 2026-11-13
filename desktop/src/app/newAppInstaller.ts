/** Installer manifest the block/buzz-app release job publishes per stable release. */
export const BUZZ_APP_DOWNLOADS_MANIFEST_URL =
  "https://raw.githubusercontent.com/block/buzz-app/download-feed/downloads.json";
export const BUZZ_APP_DOWNLOAD_PATH_PREFIX =
  "/block/buzz-app/releases/download/";
export const NEW_APP_MANIFEST_TIMEOUT_MS = 5_000;

const BUZZ_APP_DOWNLOAD_ORIGIN = "https://github.com";

/** The file extension each manifest key must point at. */
const INSTALLER_EXTENSION_BY_TARGET: Record<string, string> = {
  "macos-arm64": ".dmg",
  "macos-x64": ".dmg",
  "windows-x64": ".exe",
  "linux-appimage": ".AppImage",
  "linux-deb": ".deb",
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/**
 * Returns the installer URL the manifest lists for `target`, or null when the
 * manifest or the entry is anything other than a stable-channel release asset
 * of block/buzz-app with the file type that target installs.
 */
export function resolveInstallerUrl(
  manifest: unknown,
  target: string,
): string | null {
  if (!Object.hasOwn(INSTALLER_EXTENSION_BY_TARGET, target)) {
    return null;
  }
  if (
    !isRecord(manifest) ||
    manifest.schemaVersion !== 1 ||
    manifest.channel !== "stable" ||
    !isRecord(manifest.downloads) ||
    !Object.hasOwn(manifest.downloads, target)
  ) {
    return null;
  }
  const entry = manifest.downloads[target];
  if (typeof entry !== "string") {
    return null;
  }

  let url: URL;
  try {
    url = new URL(entry);
  } catch {
    return null;
  }
  // Checked on the parsed URL, so `..` segments, backslashes and userinfo
  // tricks are normalised before the origin and path are compared.
  if (
    url.origin !== BUZZ_APP_DOWNLOAD_ORIGIN ||
    url.username !== "" ||
    url.password !== "" ||
    !url.pathname.startsWith(BUZZ_APP_DOWNLOAD_PATH_PREFIX) ||
    !url.pathname.endsWith(INSTALLER_EXTENSION_BY_TARGET[target])
  ) {
    return null;
  }
  return url.href;
}

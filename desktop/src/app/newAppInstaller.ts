/** Installer manifest the block/buzz-app release job publishes per stable release. */
export const BUZZ_APP_DOWNLOADS_MANIFEST_URL =
  "https://raw.githubusercontent.com/block/buzz-app/download-feed/downloads.json";
export const BUZZ_APP_DOWNLOAD_PATH_PREFIX =
  "/block/buzz-app/releases/download/";
export const NEW_APP_MANIFEST_TIMEOUT_MS = 5_000;
/** Opened in the browser when this machine's installer is not resolved. */
export const BUZZ_APP_WEBSITE_URL = "https://buzz.xyz";

const BUZZ_APP_DOWNLOAD_ORIGIN = "https://github.com";

/** The file extension each manifest key must point at. */
const INSTALLER_EXTENSION_BY_TARGET: Record<string, string> = {
  "macos-arm64": ".dmg",
  "macos-x64": ".dmg",
  "windows-x64": ".exe",
  "linux-appimage": ".AppImage",
  "linux-deb": ".deb",
};

/**
 * The backend saves the installer under the URL's last path segment, so that
 * segment must be a plain file name: letters, digits and `._+-` only.
 */
const INSTALLER_FILE_NAME = /^[A-Za-z0-9][A-Za-z0-9._+-]{0,127}$/;
/**
 * The release tag reaches GitHub as written, so it holds no percent escape
 * for GitHub's router to decode: letters, digits and `._+-/` only.
 */
const INSTALLER_TAG = /^[A-Za-z0-9._+/-]+$/;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/**
 * Returns the installer URL the manifest lists for `target`, or null when the
 * manifest or the entry is anything other than a stable-channel release asset
 * of block/buzz-app with the file type that target installs. The backend
 * applies the same URL rules again before it downloads anything.
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
    !url.pathname.endsWith(INSTALLER_EXTENSION_BY_TARGET[target]) ||
    /[?#]/.test(url.href)
  ) {
    return null;
  }
  // `<tag>/<file name>`; a tag may hold slashes, the file name may not.
  const assetPath = url.pathname.slice(BUZZ_APP_DOWNLOAD_PATH_PREFIX.length);
  const separator = assetPath.lastIndexOf("/");
  const fileName = assetPath.slice(separator + 1);
  if (
    !INSTALLER_TAG.test(assetPath.slice(0, Math.max(separator, 0))) ||
    !INSTALLER_FILE_NAME.test(fileName) ||
    fileName === INSTALLER_EXTENSION_BY_TARGET[target]
  ) {
    return null;
  }
  return url.href;
}

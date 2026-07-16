export const BUZZ_RELEASES_URL = "https://github.com/block/buzz/releases";
const BUZZ_RELEASES_API_URL =
  "https://api.github.com/repos/block/buzz/releases?per_page=10";
const CACHE_KEY = "buzz.latestDownload.v1";
const CACHE_TTL_MS = 60 * 60 * 1000;

export type BuzzDownloadPlatform = {
  operatingSystem: "linux" | "macos" | "windows" | "unknown";
  architecture: "arm64" | "x64" | "unknown";
};

type GitHubRelease = {
  draft: boolean;
  prerelease: boolean;
  assets: Array<{ name: string; browser_download_url: string }>;
};

type UserAgentData = {
  platform?: string;
  mobile?: boolean;
  getHighEntropyValues?: (
    hints: string[],
  ) => Promise<{ architecture?: string; bitness?: string }>;
};

function normalizeOperatingSystem(
  navigatorValue: Navigator,
  userAgentData?: UserAgentData,
): BuzzDownloadPlatform["operatingSystem"] {
  const userAgent = navigatorValue.userAgent.toLowerCase();
  const platform = (
    userAgentData?.platform ??
    navigatorValue.platform ??
    ""
  ).toLowerCase();

  // Compatibility tokens are treacherous: iPadOS can report MacIntel and a
  // Macintosh UA, while Android and ChromeOS expose Linux platform strings.
  // Reject non-desktop devices before admitting desktop-looking signals.
  const isIPadDesktopMode =
    platform === "macintel" && navigatorValue.maxTouchPoints > 1;
  const isUnsupportedDevice =
    userAgentData?.mobile === true ||
    isIPadDesktopMode ||
    /android|iphone|ipad|ipod|mobile|tablet|windows phone|iemobile|opera mini|opera mobi|webos|blackberry|bb10|kindle|silk|kaios|cros/.test(
      userAgent,
    );
  if (isUnsupportedDevice) return "unknown";

  if (
    platform === "macos" ||
    platform.startsWith("mac") ||
    userAgent.includes("macintosh")
  )
    return "macos";
  if (
    platform === "windows" ||
    platform.startsWith("win") ||
    userAgent.includes("windows nt")
  )
    return "windows";
  if (
    platform === "linux" ||
    platform.startsWith("linux") ||
    userAgent.includes("linux")
  )
    return "linux";
  return "unknown";
}

function normalizeArchitecture(
  value: string,
): BuzzDownloadPlatform["architecture"] {
  const normalized = value.toLowerCase();
  if (/arm|aarch64/.test(normalized)) return "arm64";
  if (/x86|x64|amd64|64/.test(normalized)) return "x64";
  return "unknown";
}

export async function detectBuzzDownloadPlatform(
  navigatorValue: Navigator,
): Promise<BuzzDownloadPlatform> {
  const userAgentData = (
    navigatorValue as Navigator & { userAgentData?: UserAgentData }
  ).userAgentData;
  const operatingSystem = normalizeOperatingSystem(
    navigatorValue,
    userAgentData,
  );
  let architecture = normalizeArchitecture(navigatorValue.userAgent);

  if (userAgentData?.getHighEntropyValues) {
    try {
      const values = await userAgentData.getHighEntropyValues([
        "architecture",
        "bitness",
      ]);
      architecture = normalizeArchitecture(
        `${values.architecture ?? ""} ${values.bitness ?? ""}`,
      );
    } catch {
      // Privacy settings may reject high-entropy client hints. The matcher
      // below applies the safest compatible fallback for the detected OS.
    }
  }

  return { operatingSystem, architecture };
}

function assetPattern(platform: BuzzDownloadPlatform): RegExp | undefined {
  switch (platform.operatingSystem) {
    case "macos":
      // Safari withholds CPU architecture and reports MacIntel on Apple
      // Silicon. The Intel build remains compatible there through Rosetta.
      return platform.architecture === "arm64"
        ? /_aarch64\.dmg$/i
        : /_x64\.dmg$/i;
    case "windows":
      return /_x64-setup[^/]*\.exe$/i;
    case "linux":
      return platform.architecture === "arm64"
        ? undefined
        : /_amd64\.AppImage$/i;
    default:
      return undefined;
  }
}

export function selectBuzzDownloadUrl(
  releases: GitHubRelease[],
  platform: BuzzDownloadPlatform,
): string | undefined {
  const pattern = assetPattern(platform);
  if (!pattern) return undefined;

  for (const release of releases) {
    if (release.draft || release.prerelease) continue;
    const asset = release.assets.find(({ name }) => pattern.test(name));
    if (asset) return asset.browser_download_url;
  }
  return undefined;
}

export async function resolveBuzzDownloadUrl(): Promise<string> {
  const platform = await detectBuzzDownloadPlatform(navigator);
  try {
    const cached = JSON.parse(sessionStorage.getItem(CACHE_KEY) ?? "null") as {
      expiresAt: number;
      platform: BuzzDownloadPlatform;
      url: string;
    } | null;
    if (
      cached &&
      cached.expiresAt > Date.now() &&
      cached.platform.operatingSystem === platform.operatingSystem &&
      cached.platform.architecture === platform.architecture
    ) {
      return cached.url;
    }
  } catch {
    // Storage is only an optimization.
  }

  try {
    const response = await fetch(BUZZ_RELEASES_API_URL, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!response.ok) return BUZZ_RELEASES_URL;
    const url = selectBuzzDownloadUrl(
      (await response.json()) as GitHubRelease[],
      platform,
    );
    if (!url) return BUZZ_RELEASES_URL;
    try {
      sessionStorage.setItem(
        CACHE_KEY,
        JSON.stringify({
          expiresAt: Date.now() + CACHE_TTL_MS,
          platform,
          url,
        }),
      );
    } catch {
      // Storage is only an optimization.
    }
    return url;
  } catch {
    return BUZZ_RELEASES_URL;
  }
}

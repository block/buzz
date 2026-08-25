export type DesktopBuildChannel = "production" | "development";

export function normalizeBusinessOrigin(value?: string): string | null;
export function buildBusinessDockCsp(
  baseCsp: string,
  configuredOrigin?: string,
): string;
export function desktopBuildChannel(args: string[]): DesktopBuildChannel;
export function validateDesktopBuildEnvironment(
  channel: DesktopBuildChannel,
  env: Record<string, string | undefined>,
): void;

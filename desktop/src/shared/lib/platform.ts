type ModifierKeyboardEvent = Pick<
  KeyboardEvent,
  "altKey" | "ctrlKey" | "metaKey" | "shiftKey"
>;

/** Returns true on macOS/iOS-style Apple platforms. */
export function isMacPlatform(): boolean {
  if (typeof navigator === "undefined") {
    return false;
  }

  return /mac|iphone|ipad|ipod/i.test(navigator.platform);
}

/**
 * Returns true on Windows desktops.
 *
 * WebView2 reports `navigator.platform` as `Win32` today, but the property is
 * deprecated, so the user agent backs it up. This gates the app-drawn caption
 * buttons on an undecorated window: a false negative would leave that window
 * with no way to close, so it does not rest on one deprecated signal.
 */
export function isWindowsPlatform(): boolean {
  if (typeof navigator === "undefined") {
    return false;
  }

  return (
    /win/i.test(navigator.platform) || /windows/i.test(navigator.userAgent)
  );
}

/** Returns true on Linux desktops (excludes Android). */
export function isLinuxPlatform(): boolean {
  if (typeof navigator === "undefined") {
    return false;
  }

  return (
    /linux/i.test(navigator.platform) && !/android/i.test(navigator.userAgent)
  );
}

/**
 * The platform's normal application-shortcut modifier:
 * - macOS: Command (Meta)
 * - Windows/Linux: Control
 *
 * On macOS this intentionally rejects Control so native Emacs-style text
 * editing shortcuts (Ctrl-A/E/B/F/K/etc.) are left available to text fields.
 */
export function hasPrimaryShortcutModifier(
  event: ModifierKeyboardEvent,
): boolean {
  if (isMacPlatform()) {
    return event.metaKey && !event.ctrlKey;
  }

  return event.ctrlKey && !event.metaKey;
}

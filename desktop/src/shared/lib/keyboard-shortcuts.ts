import { i18n } from "@/i18n";
import { isMacPlatform } from "@/shared/lib/platform";

export const HUDDLE_SHORTCUT_EVENT = "buzz:huddle-shortcut";

export type HuddleShortcutDetail = {
  channelId: string;
};

export type ShortcutCategory =
  | "Navigation"
  | "Messages"
  | "Formatting"
  | "Zoom";

/** Label + description resolved in whatever language is active right now. */
export type ShortcutText = {
  label: string;
  description: string;
};

/**
 * One row of the registry. It deliberately carries no copy: `text()` resolves
 * the strings at call time, so the list follows a live language switch instead
 * of freezing the language that happened to be active at module evaluation.
 */
export type ShortcutSpec = {
  id: string;
  text: () => ShortcutText;
  keys: string;
  keysWindows: string;
  category: ShortcutCategory;
};

/** A registry row with its copy resolved — the shape the help pane renders. */
export type KeyboardShortcut = {
  id: string;
  label: string;
  description: string;
  keys: string;
  keysWindows: string;
  category: ShortcutCategory;
};

/**
 * Source of truth for every shortcut: ids, key combos and order are the same
 * ones the help pane, the inline hints and the key handlers are built from.
 */
export const KEYBOARD_SHORTCUTS: ShortcutSpec[] = [
  // Navigation
  {
    id: "quick-search",
    text: () => ({
      label: i18n.t("shared.keyboard.quick-search.label"),
      description: i18n.t("shared.keyboard.quick-search.description"),
    }),
    keys: "⌘K",
    keysWindows: "Ctrl+K",
    category: "Navigation",
  },
  {
    id: "browse-channels",
    text: () => ({
      label: i18n.t("shared.keyboard.browse-channels.label"),
      description: i18n.t("shared.keyboard.browse-channels.description"),
    }),
    keys: "⇧⌘O",
    keysWindows: "Shift+Ctrl+O",
    category: "Navigation",
  },
  {
    id: "browse-dms",
    text: () => ({
      label: i18n.t("shared.keyboard.browse-dms.label"),
      description: i18n.t("shared.keyboard.browse-dms.description"),
    }),
    keys: "⇧⌘K",
    keysWindows: "Shift+Ctrl+K",
    category: "Navigation",
  },
  {
    id: "new-channel",
    text: () => ({
      label: i18n.t("shared.keyboard.new-channel.label"),
      description: i18n.t("shared.keyboard.new-channel.description"),
    }),
    keys: "⇧⌘N",
    keysWindows: "Shift+Ctrl+N",
    category: "Navigation",
  },
  {
    id: "open-settings",
    text: () => ({
      label: i18n.t("shared.keyboard.open-settings.label"),
      description: i18n.t("shared.keyboard.open-settings.description"),
    }),
    keys: "⌘,",
    keysWindows: "Ctrl+,",
    category: "Navigation",
  },
  {
    id: "go-back",
    text: () => ({
      label: i18n.t("shared.keyboard.go-back.label"),
      description: i18n.t("shared.keyboard.go-back.description"),
    }),
    keys: "⌘[",
    keysWindows: "Alt+←",
    category: "Navigation",
  },
  {
    id: "go-forward",
    text: () => ({
      label: i18n.t("shared.keyboard.go-forward.label"),
      description: i18n.t("shared.keyboard.go-forward.description"),
    }),
    keys: "⌘]",
    keysWindows: "Alt+→",
    category: "Navigation",
  },
  {
    id: "find-in-channel",
    text: () => ({
      label: i18n.t("shared.keyboard.find-in-channel.label"),
      description: i18n.t("shared.keyboard.find-in-channel.description"),
    }),
    keys: "⌘F",
    keysWindows: "Ctrl+F",
    category: "Navigation",
  },
  {
    id: "go-home",
    text: () => ({
      label: i18n.t("shared.keyboard.go-home.label"),
      description: i18n.t("shared.keyboard.go-home.description"),
    }),
    keys: "⇧⌘A",
    keysWindows: "Shift+Ctrl+A",
    category: "Navigation",
  },
  {
    id: "toggle-sidebar",
    text: () => ({
      label: i18n.t("shared.keyboard.toggle-sidebar.label"),
      description: i18n.t("shared.keyboard.toggle-sidebar.description"),
    }),
    keys: "⌘S",
    keysWindows: "Ctrl+S",
    category: "Navigation",
  },
  {
    id: "mark-current-read",
    text: () => ({
      label: i18n.t("shared.keyboard.mark-current-read.label"),
      description: i18n.t("shared.keyboard.mark-current-read.description"),
    }),
    keys: "Escape",
    keysWindows: "Escape",
    category: "Navigation",
  },
  {
    id: "mark-all-read",
    text: () => ({
      label: i18n.t("shared.keyboard.mark-all-read.label"),
      description: i18n.t("shared.keyboard.mark-all-read.description"),
    }),
    keys: "⇧Escape",
    keysWindows: "Shift+Escape",
    category: "Navigation",
  },

  // Zoom
  {
    id: "zoom-in",
    text: () => ({
      label: i18n.t("shared.keyboard.zoom-in.label"),
      description: i18n.t("shared.keyboard.zoom-in.description"),
    }),
    keys: "⌘+",
    keysWindows: "Ctrl+=",
    category: "Zoom",
  },
  {
    id: "zoom-out",
    text: () => ({
      label: i18n.t("shared.keyboard.zoom-out.label"),
      description: i18n.t("shared.keyboard.zoom-out.description"),
    }),
    keys: "⌘-",
    keysWindows: "Ctrl+-",
    category: "Zoom",
  },
  {
    id: "zoom-reset",
    text: () => ({
      label: i18n.t("shared.keyboard.zoom-reset.label"),
      description: i18n.t("shared.keyboard.zoom-reset.description"),
    }),
    keys: "⌘0",
    keysWindows: "Ctrl+0",
    category: "Zoom",
  },

  // Messages
  {
    id: "send-message",
    text: () => ({
      label: i18n.t("shared.keyboard.send-message.label"),
      description: i18n.t("shared.keyboard.send-message.description"),
    }),
    keys: "Enter",
    keysWindows: "Enter",
    category: "Messages",
  },
  {
    id: "new-line",
    text: () => ({
      label: i18n.t("shared.keyboard.new-line.label"),
      description: i18n.t("shared.keyboard.new-line.description"),
    }),
    keys: "Shift+Enter",
    keysWindows: "Shift+Enter",
    category: "Messages",
  },
  {
    id: "always-address-agent",
    text: () => ({
      label: i18n.t("shared.keyboard.always-address-agent.label"),
      description: i18n.t("shared.keyboard.always-address-agent.description"),
    }),
    keys: "⇧⌘M",
    keysWindows: "Ctrl+Shift+M",
    category: "Messages",
  },
  {
    id: "publish-note",
    text: () => ({
      label: i18n.t("shared.keyboard.publish-note.label"),
      description: i18n.t("shared.keyboard.publish-note.description"),
    }),
    keys: "⌘Enter",
    keysWindows: "Ctrl+Enter",
    category: "Messages",
  },
  {
    id: "close-dialog",
    text: () => ({
      label: i18n.t("shared.keyboard.close-dialog.label"),
      description: i18n.t("shared.keyboard.close-dialog.description"),
    }),
    keys: "Escape",
    keysWindows: "Escape",
    category: "Messages",
  },
  {
    id: "toggle-huddle",
    text: () => ({
      label: i18n.t("shared.keyboard.toggle-huddle.label"),
      description: i18n.t("shared.keyboard.toggle-huddle.description"),
    }),
    keys: "Ctrl+Shift+Space",
    keysWindows: "Ctrl+Shift+Space",
    category: "Messages",
  },
  {
    id: "push-to-talk",
    text: () => ({
      label: i18n.t("shared.keyboard.push-to-talk.label"),
      description: i18n.t("shared.keyboard.push-to-talk.description"),
    }),
    keys: "Ctrl+Space",
    keysWindows: "Ctrl+Space",
    category: "Messages",
  },

  // Formatting
  {
    id: "format-bold",
    text: () => ({
      label: i18n.t("shared.keyboard.format-bold.label"),
      description: i18n.t("shared.keyboard.format-bold.description"),
    }),
    keys: "⌘B",
    keysWindows: "Ctrl+B",
    category: "Formatting",
  },
  {
    id: "format-italic",
    text: () => ({
      label: i18n.t("shared.keyboard.format-italic.label"),
      description: i18n.t("shared.keyboard.format-italic.description"),
    }),
    keys: "⌘I",
    keysWindows: "Ctrl+I",
    category: "Formatting",
  },
  {
    id: "format-strikethrough",
    text: () => ({
      label: i18n.t("shared.keyboard.format-strikethrough.label"),
      description: i18n.t("shared.keyboard.format-strikethrough.description"),
    }),
    keys: "⌘⇧X",
    keysWindows: "Ctrl+Shift+X",
    category: "Formatting",
  },
  {
    id: "format-code",
    text: () => ({
      label: i18n.t("shared.keyboard.format-code.label"),
      description: i18n.t("shared.keyboard.format-code.description"),
    }),
    keys: "⌘E",
    keysWindows: "Ctrl+E",
    category: "Formatting",
  },
  {
    id: "format-link",
    text: () => ({
      label: i18n.t("shared.keyboard.format-link.label"),
      description: i18n.t("shared.keyboard.format-link.description"),
    }),
    keys: "⌘K",
    keysWindows: "Ctrl+K",
    category: "Formatting",
  },
];

/**
 * Both platform spellings of a combo, so `getPlatformKeys` accepts a raw
 * registry row and a resolved one alike.
 */
export type ShortcutKeys = Pick<ShortcutSpec, "keys" | "keysWindows">;

/** Registry groups in display order, with the heading resolved at call time. */
const CATEGORY_ORDER: {
  category: ShortcutCategory;
  title: () => string;
}[] = [
  {
    category: "Navigation",
    title: () => i18n.t("shared.keyboard.category.navigation"),
  },
  {
    category: "Messages",
    title: () => i18n.t("shared.keyboard.category.messages"),
  },
  {
    category: "Formatting",
    title: () => i18n.t("shared.keyboard.category.formatting"),
  },
  {
    category: "Zoom",
    title: () => i18n.t("shared.keyboard.category.zoom"),
  },
];

/** A registry row with its copy resolved for the language active right now. */
function resolveShortcutText(spec: ShortcutSpec): KeyboardShortcut {
  const { label, description } = spec.text();
  return {
    id: spec.id,
    label,
    description,
    keys: spec.keys,
    keysWindows: spec.keysWindows,
    category: spec.category,
  };
}

/**
 * The help pane's groups, with every label resolved from the language active at
 * call time — nothing is memoized, so switching language cannot leave stale copy
 * behind. The map key is the group's display title, which is what the pane
 * renders as its heading; group order stays the registry's, not the locale's.
 */
export function getShortcutsByCategory(): Map<string, KeyboardShortcut[]> {
  const map = new Map<string, KeyboardShortcut[]>();
  for (const { category, title } of CATEGORY_ORDER) {
    map.set(
      title(),
      KEYBOARD_SHORTCUTS.filter((s) => s.category === category).map(
        resolveShortcutText,
      ),
    );
  }
  return map;
}

export function getPlatformKeys(shortcut: ShortcutKeys): string {
  return isMacPlatform() ? shortcut.keys : shortcut.keysWindows;
}

/**
 * Platform-appropriate key hint for a shortcut in {@link KEYBOARD_SHORTCUTS},
 * or null when the id is unknown. Use this for inline hints (menus, tooltips)
 * so they stay in sync with the canonical shortcut registry.
 */
export function getPlatformKeysById(id: string): string | null {
  const shortcut = KEYBOARD_SHORTCUTS.find((s) => s.id === id);
  return shortcut ? getPlatformKeys(shortcut) : null;
}

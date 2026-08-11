import {
  getShortcutsByCategory,
  getPlatformKeys,
  type KeyboardShortcut,
} from "@/shared/lib/keyboard-shortcuts";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

// Split a key-combo string on "+" separators. A "+" is a separator unless it
// directly follows another "+" (so "⌘++" keeps "+" as a key) or only
// whitespace remains after it (so a trailing "+" stays attached). Mirrors the
// previous /(?<!\+)\+(?!\s*$)/ split without lookbehind/lookahead.
function splitKeyCombo(keys: string): string[] {
  const parts: string[] = [];
  let current = "";
  for (let i = 0; i < keys.length; i++) {
    const ch = keys[i];
    const isSeparator =
      ch === "+" && keys[i - 1] !== "+" && keys.slice(i + 1).trim() !== "";
    if (isSeparator) {
      parts.push(current);
      current = "";
    } else {
      current += ch;
    }
  }
  parts.push(current);
  return parts;
}

function KeyCombo({ shortcut }: { shortcut: KeyboardShortcut }) {
  const keys = getPlatformKeys(shortcut);
  // Split on "+" but keep "+" as a standalone key (e.g. for zoom-in "⌘+").
  // Implemented without RegExp lookbehind so the bundle parses on macOS 12's
  // system WebKit (Safari 16.4 feature). See block/buzz#3295.
  const parts = splitKeyCombo(keys)
    .map((p) => p.trim())
    .filter(Boolean);

  return (
    <span className="flex items-center gap-1">
      {parts.map((part) => (
        <kbd
          className="inline-flex h-6 min-w-6 items-center justify-center rounded border border-border/70 bg-muted/60 px-1.5 font-mono text-xs text-muted-foreground"
          key={part}
        >
          {part}
        </kbd>
      ))}
    </span>
  );
}

export function KeyboardShortcutsCard() {
  const categories = getShortcutsByCategory();

  return (
    <section className="min-w-0" data-testid="settings-shortcuts">
      <SettingsSectionHeader
        title="Keyboard shortcuts"
        description="All available keyboard shortcuts. Shortcuts are read-only."
      />

      <div className="space-y-4">
        {[...categories.entries()].map(([category, shortcuts]) => (
          <div key={category}>
            <h2 className="mb-2 text-lg font-semibold tracking-tight">
              {category}
            </h2>
            <SettingsOptionGroup>
              {shortcuts.map((shortcut) => (
                <SettingsOptionRow
                  className="min-h-12 px-3 py-2"
                  key={shortcut.id}
                >
                  <div className="min-w-0 flex-1">
                    <span className="text-sm font-medium text-foreground">
                      {shortcut.label}
                    </span>
                    <span className="ml-2 text-muted-foreground">
                      {shortcut.description}
                    </span>
                  </div>
                  <KeyCombo shortcut={shortcut} />
                </SettingsOptionRow>
              ))}
            </SettingsOptionGroup>
          </div>
        ))}
      </div>
    </section>
  );
}

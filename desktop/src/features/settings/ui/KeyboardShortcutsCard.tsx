import {
  getShortcutsByCategory,
  getPlatformKeys,
  type KeyboardShortcut,
} from "@/shared/lib/keyboard-shortcuts";
import {
  setComposerSubmitShortcut,
  useComposerSubmitShortcut,
  type ComposerSubmitShortcut,
} from "@/features/messages/lib/composerSubmitShortcut";
import {
  SettingsOptionGroup,
  SettingsOptionGroupList,
  SettingsOptionRow,
} from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

const MOD_ENTER_SHORTCUT: KeyboardShortcut = {
  id: "composer-mod-enter",
  label: "",
  description: "",
  keys: "⌘Enter",
  keysWindows: "Ctrl+Enter",
  category: "Messages",
};

function getShortcutForDisplay(
  shortcut: KeyboardShortcut,
  composerSubmitShortcut: ComposerSubmitShortcut,
): KeyboardShortcut {
  if (
    shortcut.id === "send-message" &&
    composerSubmitShortcut === "mod-enter"
  ) {
    return { ...shortcut, keys: "⌘Enter", keysWindows: "Ctrl+Enter" };
  }
  if (shortcut.id === "new-line" && composerSubmitShortcut === "mod-enter") {
    return { ...shortcut, keys: "Enter", keysWindows: "Enter" };
  }
  return shortcut;
}

function KeyCombo({ shortcut }: { shortcut: KeyboardShortcut }) {
  const keys = getPlatformKeys(shortcut);
  // Split on "+" but keep "+" as a standalone key (e.g. for zoom-in "⌘+")
  const parts = keys
    .split(/(?<!\+)\+(?!\s*$)/)
    .map((p) => p.trim())
    .filter(Boolean);

  return (
    <span className="flex items-center gap-1">
      {parts.map((part) => (
        <InlineKey key={part}>{part}</InlineKey>
      ))}
    </span>
  );
}

function InlineKey({ children }: { children: string }) {
  return (
    <kbd className="inline-flex h-6 min-w-6 items-center justify-center rounded border border-border/70 bg-muted/60 px-1.5 font-mono text-xs text-muted-foreground">
      {children}
    </kbd>
  );
}

function InlineSendShortcut() {
  const keys = getPlatformKeys(MOD_ENTER_SHORTCUT);
  const parts = keys === "⌘Enter" ? ["⌘", "Enter"] : keys.split("+");

  return (
    <span className="inline-flex items-center gap-1 align-middle">
      {parts.map((part) => (
        <InlineKey key={part}>{part}</InlineKey>
      ))}
    </span>
  );
}

function ComposerSubmitShortcutRow({
  sendWithModEnter,
}: {
  sendWithModEnter: boolean;
}) {
  return (
    <SettingsOptionRow className="min-h-0 flex-col items-stretch justify-start gap-2 px-3 py-3">
      <fieldset data-testid="composer-submit-shortcut-options">
        <legend className="flex flex-wrap items-center gap-1.5 text-sm font-medium text-foreground">
          <span>When writing a message, press</span>
          <InlineKey>Enter</InlineKey>
          <span>to...</span>
        </legend>
        <div className="mt-2 flex flex-col gap-2 sm:flex-row sm:items-center sm:gap-6">
          <label
            className="flex cursor-pointer items-center gap-2 text-sm text-foreground"
            htmlFor="composer-submit-shortcut-enter"
          >
            <input
              checked={!sendWithModEnter}
              className="h-4 w-4 accent-primary"
              id="composer-submit-shortcut-enter"
              name="composer-submit-shortcut"
              onChange={() => setComposerSubmitShortcut("enter")}
              type="radio"
            />
            <span>Send the message</span>
          </label>
          <label
            className="flex cursor-pointer items-center gap-2 text-sm text-foreground"
            htmlFor="composer-submit-shortcut-mod-enter"
          >
            <input
              checked={sendWithModEnter}
              className="h-4 w-4 accent-primary"
              id="composer-submit-shortcut-mod-enter"
              name="composer-submit-shortcut"
              onChange={() => setComposerSubmitShortcut("mod-enter")}
              type="radio"
            />
            <span className="flex flex-wrap items-center gap-1.5">
              <span>Start a new line</span>
              <span className="text-muted-foreground">(use</span>
              <InlineSendShortcut />
              <span className="text-muted-foreground">to send)</span>
            </span>
          </label>
        </div>
      </fieldset>
    </SettingsOptionRow>
  );
}

export function KeyboardShortcutsCard() {
  const categories = getShortcutsByCategory();
  const composerSubmitShortcut = useComposerSubmitShortcut();
  const sendWithModEnter = composerSubmitShortcut === "mod-enter";

  return (
    <section className="min-w-0" data-testid="settings-shortcuts">
      <SettingsSectionHeader
        title="Keyboard shortcuts"
        description="All available keyboard shortcuts."
      />

      <SettingsOptionGroupList>
        {[...categories.entries()].map(([category, shortcuts]) => (
          <SettingsOptionGroup key={category} title={category}>
            {category === "Messages" ? (
              <ComposerSubmitShortcutRow sendWithModEnter={sendWithModEnter} />
            ) : null}
            {shortcuts.map((shortcut) => (
              <SettingsOptionRow
                className="min-h-12 px-3 py-2"
                key={shortcut.id}
              >
                <div className="min-w-0 flex-1">
                  <span className="text-sm font-medium text-foreground">
                    {shortcut.label}
                  </span>
                  <span
                    className="ml-2 text-muted-foreground/70"
                    data-settings-subcopy
                  >
                    {shortcut.description}
                  </span>
                </div>
                <KeyCombo
                  shortcut={getShortcutForDisplay(
                    shortcut,
                    composerSubmitShortcut,
                  )}
                />
              </SettingsOptionRow>
            ))}
          </SettingsOptionGroup>
        ))}
      </SettingsOptionGroupList>
    </section>
  );
}

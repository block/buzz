import {
  getShortcutsByCategory,
  getPlatformKeys,
  type KeyboardShortcut,
} from "@/shared/lib/keyboard-shortcuts";
import { Switch } from "@/shared/ui/switch";
import { usePttShortcutSettings } from "@/features/huddle/HuddleContext";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

function KeyCombo({ keys }: { keys: string }) {
  // Split on "+" but keep "+" as a standalone key (e.g. for zoom-in "⌘+")
  const parts = keys
    .split(/(?<!\+)\+(?!\s*$)/)
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

function ShortcutKeyCombo({ shortcut }: { shortcut: KeyboardShortcut }) {
  return <KeyCombo keys={getPlatformKeys(shortcut)} />;
}

function PushToTalkGlobalShortcutRow() {
  const { settings, setEnabled, isUpdating } = usePttShortcutSettings();
  const status = settings.enabled
    ? settings.registered
      ? "Active while this PTT huddle is open."
      : "Registered only while you are in a huddle with Push to Talk selected."
    : "Disabled. Push to Talk remains available, but no app-wide shortcut is reserved.";

  return (
    <SettingsOptionGroup>
      <SettingsOptionRow className="items-start px-3 py-3">
        <div className="min-w-0 flex-1">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="ptt-global-shortcut-switch"
          >
            Push to Talk
          </label>
          <p className="mt-1 text-sm text-muted-foreground">
            Hold the shortcut to transmit in huddles when Push to Talk is
            selected.
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            Global shortcuts work while Buzz is in the background and may
            override shortcuts in other apps.
          </p>
          <p className="mt-2 text-xs text-muted-foreground">{status}</p>
          {settings.error ? (
            <p className="mt-2 text-xs text-destructive" role="alert">
              {settings.error}
            </p>
          ) : null}
        </div>
        <div className="flex shrink-0 items-center gap-3 pt-0.5">
          <KeyCombo keys={settings.display} />
          <Switch
            checked={settings.enabled}
            data-testid="ptt-global-shortcut-toggle"
            disabled={isUpdating}
            id="ptt-global-shortcut-switch"
            onCheckedChange={(checked) => {
              void setEnabled(checked);
            }}
          />
        </div>
      </SettingsOptionRow>
    </SettingsOptionGroup>
  );
}

export function KeyboardShortcutsCard() {
  const categories = getShortcutsByCategory();

  return (
    <section className="min-w-0" data-testid="settings-shortcuts">
      <SettingsSectionHeader
        title="Keyboard Shortcuts"
        description="View Buzz shortcuts and control the global shortcuts that can work outside the app."
      />

      <div className="space-y-4">
        <div>
          <h3 className="mb-2 text-xs font-semibold uppercase tracking-widest text-muted-foreground">
            Global shortcuts
          </h3>
          <PushToTalkGlobalShortcutRow />
        </div>

        {[...categories.entries()].map(([category, shortcuts]) => (
          <div key={category}>
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-widest text-muted-foreground">
              {category}
            </h3>
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
                  <ShortcutKeyCombo shortcut={shortcut} />
                </SettingsOptionRow>
              ))}
            </SettingsOptionGroup>
          </div>
        ))}
      </div>
    </section>
  );
}

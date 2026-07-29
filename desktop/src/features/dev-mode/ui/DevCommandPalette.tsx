import * as React from "react";
import { useNavigate } from "@tanstack/react-router";

import {
  AUTHOR_COLOR_PALETTE,
  defaultAuthorColor,
  normalizeHexColor,
  setNameColorOverride,
} from "@/features/dev-mode/lib/authorColors";
import { setDisplayStyle } from "@/features/dev-mode/lib/displayStylePreference";
import type { SettingsSection } from "@/features/settings/ui/SettingsPanels";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

type PaletteEntry = {
  id: string;
  label: string;
  detail?: string;
  /** Swatch color for color-picker entries. */
  swatch?: string;
  run: () => void;
};

type PaletteMode = "root" | "color";

const SETTINGS_ENTRIES: { section: SettingsSection; label: string }[] = [
  { section: "agents", label: "configure agents" },
  { section: "appearance", label: "appearance settings" },
  { section: "profile", label: "profile settings" },
  { section: "notifications", label: "notification settings" },
  { section: "shortcuts", label: "keyboard shortcuts" },
  { section: "experimental", label: "experimental features" },
  { section: "channel-templates", label: "channel templates" },
  { section: "compute", label: "compute settings" },
  { section: "updates", label: "check for updates" },
];

/**
 * Amp-style command palette for developer mode: channel search across every
 * session plus management/configuration actions. Opened with Ctrl+O anywhere
 * in the shell, or `/` in an empty composer.
 */
export function DevCommandPalette({
  channels,
  myPubkey,
  onOpenChannel,
  onNewSession,
  onClose,
}: {
  /** All session channels, newest first. */
  channels: Channel[];
  myPubkey: string | null;
  onOpenChannel: (channelId: string) => void;
  onNewSession: () => void;
  onClose: () => void;
}) {
  const navigate = useNavigate();
  const [query, setQuery] = React.useState("");
  const [mode, setMode] = React.useState<PaletteMode>("root");
  const [selectedIndex, setSelectedIndex] = React.useState(0);
  const inputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const openSettings = React.useCallback(
    (section: SettingsSection) => {
      onClose();
      void navigate({ to: "/settings", search: { section } });
    },
    [navigate, onClose],
  );

  const entries = React.useMemo<PaletteEntry[]>(() => {
    const needle = query.trim().toLowerCase();

    if (mode === "color") {
      const colorEntries: PaletteEntry[] = AUTHOR_COLOR_PALETTE.map(
        (color) => ({
          id: `color-${color}`,
          label: color,
          swatch: color,
          run: () => {
            if (myPubkey) setNameColorOverride(myPubkey, color);
            onClose();
          },
        }),
      );
      const typed = normalizeHexColor(needle);
      if (typed) {
        colorEntries.unshift({
          id: "color-custom",
          label: `use ${typed}`,
          swatch: typed,
          run: () => {
            if (myPubkey) setNameColorOverride(myPubkey, typed);
            onClose();
          },
        });
      }
      colorEntries.push({
        id: "color-reset",
        label: "reset to default",
        swatch: myPubkey ? defaultAuthorColor(myPubkey) : undefined,
        run: () => {
          if (myPubkey) setNameColorOverride(myPubkey, null);
          onClose();
        },
      });
      return typed
        ? colorEntries
        : colorEntries.filter((entry) =>
            entry.label.toLowerCase().includes(needle),
          );
    }

    const actions: PaletteEntry[] = [
      {
        id: "new-session",
        label: "new session",
        detail: "fresh prompt",
        run: () => {
          onNewSession();
          onClose();
        },
      },
      {
        id: "standard-ui",
        label: "switch to standard ui",
        detail: "⌘⇧D",
        run: () => {
          onClose();
          setDisplayStyle("standard");
        },
      },
      {
        id: "name-color",
        label: "set my name color",
        detail: "hex or preset",
        run: () => {
          setMode("color");
          setQuery("");
          setSelectedIndex(0);
        },
      },
      ...SETTINGS_ENTRIES.map(
        (entry): PaletteEntry => ({
          id: `settings-${entry.section}`,
          label: entry.label,
          detail: "settings",
          run: () => openSettings(entry.section),
        }),
      ),
    ];

    const channelEntries: PaletteEntry[] = channels.map((channel) => ({
      id: `channel-${channel.id}`,
      label: `# ${channel.name}`,
      detail: channel.description ?? undefined,
      run: () => {
        onOpenChannel(channel.id);
        onClose();
      },
    }));

    const all = [...actions, ...channelEntries];
    if (!needle) return all;
    return all.filter((entry) =>
      `${entry.label} ${entry.detail ?? ""}`.toLowerCase().includes(needle),
    );
  }, [
    channels,
    mode,
    myPubkey,
    onClose,
    onNewSession,
    onOpenChannel,
    openSettings,
    query,
  ]);

  const clampedIndex = Math.min(selectedIndex, Math.max(0, entries.length - 1));

  const scrollSelectedIntoView = React.useCallback(
    (node: HTMLButtonElement | null) => {
      node?.scrollIntoView({ block: "nearest" });
    },
    [],
  );

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      if (mode === "color") {
        setMode("root");
        setQuery("");
        setSelectedIndex(0);
        return;
      }
      onClose();
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setSelectedIndex(Math.min(clampedIndex + 1, entries.length - 1));
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      setSelectedIndex(Math.max(clampedIndex - 1, 0));
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      entries[clampedIndex]?.run();
    }
  };

  return (
    <div
      className="absolute inset-0 z-50 flex items-start justify-center pt-24 font-mono"
      data-testid="dev-mode-palette"
    >
      <div
        aria-hidden="true"
        className="absolute inset-0 bg-background/60"
        onClick={onClose}
      />
      <div className="relative flex max-h-[60vh] w-[560px] flex-col border border-border bg-background shadow-lg">
        <input
          ref={inputRef}
          className="shrink-0 border-b border-border/60 bg-transparent px-3 py-2 text-sm outline-none placeholder:text-muted-foreground/60"
          data-testid="dev-mode-palette-input"
          onChange={(event) => {
            setQuery(event.target.value);
            setSelectedIndex(0);
          }}
          onKeyDown={handleKeyDown}
          placeholder={
            mode === "color"
              ? "type a hex color or pick a preset…"
              : "search channels and commands…"
          }
          spellCheck={false}
          value={query}
        />
        <div className="min-h-0 flex-1 overflow-y-auto py-1">
          {entries.length === 0 ? (
            <div className="px-3 py-2 text-sm text-muted-foreground/60">
              no matches
            </div>
          ) : null}
          {entries.map((entry, index) => (
            <button
              key={entry.id}
              ref={index === clampedIndex ? scrollSelectedIntoView : undefined}
              className={cn(
                "flex w-full cursor-pointer items-baseline gap-2 px-3 py-1 text-left text-sm",
                index === clampedIndex
                  ? "bg-primary/15 text-foreground"
                  : "text-muted-foreground hover:bg-muted/40 hover:text-foreground",
              )}
              onClick={entry.run}
              onMouseMove={() => setSelectedIndex(index)}
              type="button"
            >
              {entry.swatch ? (
                <span
                  aria-hidden
                  className="inline-block size-3 shrink-0 self-center border border-border/60"
                  style={{ backgroundColor: entry.swatch }}
                />
              ) : null}
              <span className="min-w-0 flex-1 truncate">{entry.label}</span>
              {entry.detail ? (
                <span className="max-w-48 shrink-0 truncate text-xs text-muted-foreground/60">
                  {entry.detail}
                </span>
              ) : null}
            </button>
          ))}
        </div>
        <div className="shrink-0 border-t border-border/60 px-3 py-1.5 text-xs text-muted-foreground/60">
          ↑↓: select · enter: run · esc: {mode === "color" ? "back" : "close"}
        </div>
      </div>
    </div>
  );
}

// Phone-only tab switcher: a floating round button (top-left, under the safe area) with the open-tab count, which
// fans out one list of all surface tabs (channels, DMs, threads, pages) and Buzz Term tabs. Uses the existing
// SurfaceTabsProvider and TerminalBootstrap handlers; no separate tab state.
import { Hash, MessageCircle, MessagesSquare, PanelTop, SquareTerminal, X } from "lucide-react";
import * as React from "react";

import { setPhoneOverlay } from "@/app/phone/phoneTabs";
import { useOptionalSurfaceTabs } from "@/app/surfaceTabs/SurfaceTabsProvider";
import type { SurfaceTab } from "@/app/surfaceTabs/surfaceTabModel";
import { useTerminalTabs } from "@/features/terminal/terminalTabsBridge";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

type Row = {
  key: string;
  label: string;
  type: "Channel" | "DM" | "Thread" | "Terminal" | "Page";
  active: boolean;
  open: () => void;
  close: () => void;
};

const ICONS = { Channel: Hash, DM: MessageCircle, Thread: MessagesSquare, Terminal: SquareTerminal, Page: PanelTop };

export function PhoneTabSwitcher({
  channels,
  onOpenBuzzTerm,
  terminalActive,
}: {
  channels: readonly Channel[];
  onOpenBuzzTerm: () => void;
  /** True while the Buzz Term screen is showing (its active session is the current tab). */
  terminalActive: boolean;
}) {
  const surface = useOptionalSurfaceTabs();
  const term = useTerminalTabs();
  const [open, setOpen] = React.useState(false);
  const panelRef = React.useRef<HTMLDivElement>(null);
  const buttonRef = React.useRef<HTMLButtonElement>(null);

  const surfaceTabs = surface?.tabs ?? [];
  const typeOf = (tab: SurfaceTab): Row["type"] => {
    if (tab.target.kind === "thread") return "Thread";
    if (tab.target.kind === "channel") {
      const id = tab.target.channelId;
      return channels.find((c) => c.id === id)?.channelType === "dm" ? "DM" : "Channel";
    }
    return "Page";
  };
  const rows: Row[] = [
    ...surfaceTabs.map((tab, i) => ({
      key: `s:${tab.id}`,
      label: tab.label,
      type: typeOf(tab),
      active: !terminalActive && surface?.activeId === tab.id,
      open: () => {
        setPhoneOverlay(null); // leave the Channels/Me/Search list screens for the tab's surface
        surface?.selectTab(tab.id);
      },
      close: () => {
        // Same close as the desktop strip; closing the current tab moves to the next one (or the previous).
        const wasActive = surface?.activeId === tab.id;
        surface?.closeTab(tab.id);
        const next = surfaceTabs[i + 1] ?? surfaceTabs[i - 1];
        if (wasActive && next) {
          setPhoneOverlay(null);
          surface?.selectTab(next.id);
        }
      },
    })),
    ...term.tabs.map((tab, i) => ({
      key: `t:${tab.id}`,
      label: tab.title || tab.channelName || "Terminal",
      type: "Terminal" as const,
      active: terminalActive && tab.active,
      open: () => {
        onOpenBuzzTerm();
        term.select(tab.id);
      },
      close: () => {
        const next = term.tabs[i + 1] ?? term.tabs[i - 1];
        term.close(tab.id);
        if (tab.active && next) term.select(next.id);
      },
    })),
  ];
  const count = rows.length;

  React.useEffect(() => {
    const root = document.documentElement;
    if (count > 0) root.setAttribute("data-phone-tab-switcher", "");
    else root.removeAttribute("data-phone-tab-switcher");
    return () => root.removeAttribute("data-phone-tab-switcher");
  }, [count]);
  React.useEffect(() => {
    if (count === 0) setOpen(false);
  }, [count]);
  React.useEffect(() => {
    if (!open) return;
    const onDown = (e: Event) => {
      const t = e.target as Node | null;
      if (panelRef.current?.contains(t) || buttonRef.current?.contains(t)) return;
      setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("pointerdown", onDown, true);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("pointerdown", onDown, true);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [open]);

  if (count === 0) return null;
  return (
    <>
      <button
        aria-expanded={open}
        aria-label={`${count} open tab${count === 1 ? "" : "s"}`}
        className="phone-tab-switcher-button"
        data-testid="phone-tab-switcher"
        onClick={() => setOpen((v) => !v)}
        ref={buttonRef}
        type="button"
      >
        {count > 99 ? "99+" : count}
      </button>
      {open ? (
        <div className="phone-tab-switcher-panel" data-testid="phone-tab-switcher-list" ref={panelRef} role="menu">
          {rows.map((row) => {
            const Icon = ICONS[row.type];
            return (
              <div
                className={cn("phone-tab-switcher-row", row.active && "is-active")}
                data-active={row.active ? "true" : "false"}
                data-testid={`phone-tab-row-${row.key}`}
                key={row.key}
              >
                <button
                  className="flex min-w-0 flex-1 items-center gap-2.5 px-3 py-2.5 text-left"
                  data-testid="phone-tab-row-open"
                  onClick={() => {
                    setOpen(false);
                    row.open();
                  }}
                  role="menuitem"
                  type="button"
                >
                  <Icon className="h-4 w-4 shrink-0 opacity-70" />
                  <span className="min-w-0 flex-1 truncate text-sm">{row.label}</span>
                  <span className="shrink-0 text-[10px] uppercase tracking-wide opacity-60">{row.type}</span>
                </button>
                <button
                  aria-label={`Close ${row.label}`}
                  className="grid h-9 w-9 shrink-0 place-items-center opacity-70"
                  data-testid="phone-tab-row-close"
                  onClick={row.close}
                  type="button"
                >
                  <X className="h-4 w-4" />
                </button>
              </div>
            );
          })}
        </div>
      ) : null}
    </>
  );
}

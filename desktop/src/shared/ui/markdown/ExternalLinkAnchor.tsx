import { normalizeExternalHref } from "@/shared/lib/externalLinks";
import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { openLinkSidePanel } from "@/features/link-panel/lib/linkSidePanelStore";
import { usePinnedSites } from "@/features/pinned-sites/hooks";
import { matchPinnedSiteForUrl } from "@/features/pinned-sites/lib/matchPinnedSite";
import { queuePinnedSiteOpenUrl } from "@/features/pinned-sites/lib/pendingPinOpen";
import { toast } from "sonner";

import { cn } from "@/shared/lib/cn";
import { copyTextToClipboard } from "@/shared/lib/clipboard";

import { MaskedLinkTooltip } from "./MaskedLinkTooltip";
import {
  MediaContextMenu,
  type MediaContextMenuPosition,
  useDismissMediaContextMenu,
} from "./MediaContextMenu";

/**
 * An external `[text](href)` link with a custom right-click menu.
 *
 * Left-click: if the URL matches a Settings pinned website (domain / URL
 * prefix), open into that pin\'s browser session; else RHS slide-out.
 * Right-click: optional "Open in Pinned Website", then Open link (RHS),
 * Open in browser, Copy link.
 */
const IS_WEB = import.meta.env?.VITE_BUZZ_PLATFORM === "web";

export function ExternalLinkAnchor({
  anchorProps,
  children,
  href: rawHref,
  isLinearLink,
  label,
}: {
  anchorProps: React.ComponentPropsWithoutRef<"a">;
  children: React.ReactNode;
  href: string | undefined;
  isLinearLink: boolean;
  label: string;
}) {
  // Web: bare domains get https://, desktop-only schemes (file:, tauri:, buzz:, blob:…) render as plain text.
  const href = IS_WEB ? (normalizeExternalHref(rawHref) ?? undefined) : rawHref;
  const [menu, setMenu] = React.useState<MediaContextMenuPosition | null>(null);
  const closeMenu = React.useCallback(() => setMenu(null), []);
  useDismissMediaContextMenu(Boolean(menu), closeMenu);
  const { pins } = usePinnedSites();
  const { goPinnedSite } = useAppNavigation();

  const matchedPin = React.useMemo(
    () => (href ? matchPinnedSiteForUrl(href, pins) : null),
    [href, pins],
  );

  const openInSidePanel = React.useCallback(() => {
    if (!href) return;
    if (!openLinkSidePanel(href)) {
      void openUrl(href).catch(() => {
        toast.error("Failed to open link");
      });
    }
  }, [href]);

  const openInPinnedWebsite = React.useCallback(() => {
    if (!href || !matchedPin) return;
    // Left-click and context-menu "Open in Pinned Website" share this exact path.
    queuePinnedSiteOpenUrl(matchedPin.id, href);
    void goPinnedSite(matchedPin.id, undefined, { openUrl: href });
  }, [goPinnedSite, href, matchedPin]);

  const openDefault = React.useCallback(() => {
    if (matchedPin) {
      openInPinnedWebsite();
      return;
    }
    openInSidePanel();
  }, [matchedPin, openInPinnedWebsite, openInSidePanel]);

  const menuItems = React.useMemo(() => {
    if (!href) return [];
    const items: { label: string; onSelect: () => void }[] = [];
    if (matchedPin) {
      items.push({
        label: "Open in Pinned Website",
        onSelect: () => {
          closeMenu();
          openInPinnedWebsite();
        },
      });
    }
    items.push(
      {
        label: "Open link",
        onSelect: () => {
          closeMenu();
          openInSidePanel();
        },
      },
      {
        label: "Open in browser",
        onSelect: () => {
          closeMenu();
          void openUrl(href).catch(() => {
            toast.error("Failed to open link");
          });
        },
      },
      {
        label: "Copy link",
        onSelect: () => {
          closeMenu();
          copyTextToClipboard(href, "Link copied to clipboard");
        },
      },
    );
    return items;
  }, [
    closeMenu,
    href,
    matchedPin,
    openInPinnedWebsite,
    openInSidePanel,
  ]);

  if (IS_WEB && !href) return <span>{children}</span>;

  const anchor = (
    <a
      {...anchorProps}
      className={cn(
        "font-medium underline underline-offset-4 transition-colors",
        isLinearLink ? "linear-link" : "text-primary hover:text-primary/80",
      )}
      href={href}
      onClick={(event) => {
        if (!href) return;
        // Keep modified clicks (new tab / window) on the OS opener path.
        if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
          return;
        }
        event.preventDefault();
        openDefault();
      }}
      onContextMenuCapture={(event) => {
        if (!href) return;
        event.preventDefault();
        setMenu({ x: event.clientX, y: event.clientY });
      }}
      rel="noopener noreferrer"
      target="_blank"
    >
      {children}
    </a>
  );

  return (
    <>
      <MaskedLinkTooltip disabled={isLinearLink} href={href} label={label}>
        {anchor}
      </MaskedLinkTooltip>
      {menu && href ? (
        <MediaContextMenu
          dataAttributes={["data-link-context-menu"]}
          items={menuItems}
          position={menu}
        />
      ) : null}
    </>
  );
}

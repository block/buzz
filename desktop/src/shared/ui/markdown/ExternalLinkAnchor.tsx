import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
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
 * Buzz renders inside a native webview whose default context menu has no
 * useful link actions, so a plain right-click on a link is a no-op. This adds
 * an in-app menu with "Open link" (via the OS opener, matching the anchor's
 * left-click `target="_blank"` behavior) and "Copy link" (the real href, not
 * the masked display text).
 */
export function ExternalLinkAnchor({
  anchorProps,
  children,
  href,
  isLinearLink,
  label,
}: {
  anchorProps: React.ComponentPropsWithoutRef<"a">;
  children: React.ReactNode;
  href: string | undefined;
  isLinearLink: boolean;
  label: string;
}) {
  const [menu, setMenu] = React.useState<MediaContextMenuPosition | null>(null);
  const closeMenu = React.useCallback(() => setMenu(null), []);
  useDismissMediaContextMenu(Boolean(menu), closeMenu);

  // `buzzDeepLinkUrlTransform` delegates anything it does not recognise to
  // react-markdown's `defaultUrlTransform`, which replaces a disallowed scheme
  // (`obsidian://`, `javascript:`, an unrecognised `buzz://` verb, …) with the
  // empty string. Rendering that as an anchor leaves an underlined,
  // link-coloured label that navigates nowhere on click, and the context menu
  // below bails on a falsy href — so "Copy link" cannot recover the URL
  // either. Render the label as inert text so it does not claim to be a link.
  if (!href) {
    return <span className="font-medium text-current">{children}</span>;
  }

  const anchor = (
    <a
      {...anchorProps}
      className={cn(
        "font-medium underline underline-offset-4 transition-colors",
        isLinearLink ? "linear-link" : "text-primary hover:text-primary/80",
      )}
      href={href}
      onContextMenuCapture={(event) => {
        if (!href) return;
        event.preventDefault();
        setMenu({ x: event.clientX, y: event.clientY });
      }}
      rel="noreferrer"
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
          items={[
            {
              label: "Open link",
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
          ]}
          position={menu}
        />
      ) : null}
    </>
  );
}

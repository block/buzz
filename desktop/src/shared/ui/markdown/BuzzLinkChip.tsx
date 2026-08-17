import * as React from "react";

import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { InlineChip } from "@/shared/ui/InlineChip";
import type { InlineChipIconKind } from "@/shared/ui/mentionChip";

import {
  MediaContextMenu,
  type MediaContextMenuPosition,
  useDismissMediaContextMenu,
} from "./MediaContextMenu";

function useBuzzLinkContextMenu({
  href,
  interactive,
  onOpenLink,
}: {
  href: string | undefined;
  interactive: boolean;
  onOpenLink: () => void;
}) {
  const [position, setPosition] =
    React.useState<MediaContextMenuPosition | null>(null);
  const closeMenu = React.useCallback(() => setPosition(null), []);
  useDismissMediaContextMenu(Boolean(position), closeMenu);

  const onContextMenuCapture = React.useCallback(
    (event: React.MouseEvent<HTMLElement>) => {
      if (!interactive || !href) return;
      event.preventDefault();
      setPosition({ x: event.clientX, y: event.clientY });
    },
    [href, interactive],
  );

  const contextMenu =
    position && href ? (
      <MediaContextMenu
        dataAttributes={["data-buzz-link-context-menu"]}
        items={[
          {
            label: "Open link",
            onSelect: () => {
              closeMenu();
              onOpenLink();
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
        position={position}
      />
    ) : null;

  return { contextMenu, onContextMenuCapture };
}

export function BuzzLinkChip({
  children,
  className,
  href,
  icon: Icon,
  interactive,
  onOpenLink,
  ...props
}: Omit<React.ComponentPropsWithoutRef<"button">, "onClick"> & {
  href?: string;
  icon: InlineChipIconKind;
  interactive: boolean;
  onOpenLink: () => void;
}) {
  const { contextMenu, onContextMenuCapture } = useBuzzLinkContextMenu({
    href,
    interactive,
    onOpenLink,
  });

  if (!interactive) {
    return (
      <InlineChip
        {...(props as React.HTMLAttributes<HTMLSpanElement>)}
        data-buzz-link=""
        className={className}
        icon={Icon}
      >
        {children}
      </InlineChip>
    );
  }

  return (
    <>
      <InlineChip
        {...props}
        as="button"
        data-buzz-link=""
        className={className}
        icon={Icon}
        interactive
        onClick={onOpenLink}
        onContextMenuCapture={onContextMenuCapture}
      >
        {children}
      </InlineChip>
      {contextMenu}
    </>
  );
}

export function BuzzInlineLink({
  children,
  href,
  interactive,
  onOpenLink,
  ...props
}: Omit<React.ComponentPropsWithoutRef<"button">, "onClick"> & {
  href?: string;
  interactive: boolean;
  onOpenLink: () => void;
}) {
  const contextMenuHref =
    href ?? (typeof props.title === "string" ? props.title : undefined);
  const { contextMenu, onContextMenuCapture } = useBuzzLinkContextMenu({
    href: contextMenuHref,
    interactive,
    onOpenLink,
  });

  if (!interactive) {
    return <span className="font-medium text-current">{children}</span>;
  }

  return (
    <>
      <button
        {...props}
        type="button"
        className="cursor-pointer font-medium text-primary underline underline-offset-4 transition-colors hover:text-primary/80"
        onClick={onOpenLink}
        onContextMenuCapture={onContextMenuCapture}
      >
        {children}
      </button>
      {contextMenu}
    </>
  );
}

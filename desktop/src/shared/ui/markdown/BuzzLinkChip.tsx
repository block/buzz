import * as React from "react";

import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { cn } from "@/shared/lib/cn";
import { InlineChip } from "@/shared/ui/InlineChip";
import {
  inlineChipIconClasses,
  inlineChipLeadingEnd,
  type InlineChipIconKind,
  truncateInlineChipLabel,
  WRAPPING_INLINE_CHIP_CLASSES,
} from "@/shared/ui/mentionChip";

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

function wrappingChipContent(
  children: React.ReactNode,
  icon: InlineChipIconKind,
): React.ReactNode {
  if (typeof children !== "string" || children.length === 0) return children;

  const leadingEnd = inlineChipLeadingEnd(children);
  if (!leadingEnd) {
    return (
      <>
        <span
          aria-hidden="true"
          className={cn(
            "inline-chip-leading-fragment",
            inlineChipIconClasses(icon),
          )}
        />
        {children}
      </>
    );
  }

  return (
    <>
      <span
        aria-hidden="true"
        className={cn(
          "inline-chip-leading-fragment",
          inlineChipIconClasses(icon),
        )}
      >
        {children.slice(0, leadingEnd)}
      </span>
      {children.slice(leadingEnd)}
    </>
  );
}

export function BuzzLinkChip({
  children,
  className,
  href,
  icon: Icon,
  interactive,
  onOpenLink,
  wrapping = false,
  ...props
}: Omit<React.ComponentPropsWithoutRef<"span">, "onClick"> & {
  href?: string;
  icon: InlineChipIconKind;
  interactive: boolean;
  onOpenLink: () => void;
  wrapping?: boolean;
}) {
  const { contextMenu, onContextMenuCapture } = useBuzzLinkContextMenu({
    href,
    interactive,
    onOpenLink,
  });
  const visibleChildren =
    wrapping && typeof children === "string"
      ? truncateInlineChipLabel(children)
      : children;
  const content = wrapping
    ? wrappingChipContent(visibleChildren, Icon)
    : visibleChildren;
  const chipClassName = cn(className, wrapping && WRAPPING_INLINE_CHIP_CLASSES);
  const onKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLSpanElement>) => {
      props.onKeyDown?.(event);
      if (
        event.defaultPrevented ||
        (event.key !== "Enter" && event.key !== " ")
      ) {
        return;
      }
      event.preventDefault();
      onOpenLink();
    },
    [onOpenLink, props.onKeyDown],
  );

  if (!interactive) {
    return (
      <InlineChip
        {...props}
        data-buzz-link=""
        className={chipClassName}
        icon={Icon}
      >
        {content}
      </InlineChip>
    );
  }

  return (
    <>
      <InlineChip
        {...props}
        data-buzz-link=""
        className={chipClassName}
        icon={Icon}
        interactive
        role="button"
        tabIndex={0}
        onClick={onOpenLink}
        onContextMenuCapture={onContextMenuCapture}
        onKeyDown={onKeyDown}
      >
        {content}
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

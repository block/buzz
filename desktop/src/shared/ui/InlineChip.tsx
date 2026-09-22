import type * as React from "react";

import { cn } from "@/shared/lib/cn";
import {
  inlineChipIconClasses,
  type InlineChipIconKind,
  MENTION_CHIP_BASE_CLASSES,
  MENTION_CHIP_HOVER_CLASSES,
} from "@/shared/ui/mentionChip";

type InlineChipCommonProps = {
  children: React.ReactNode;
  className?: string;
  icon: InlineChipIconKind;
  interactive?: boolean;
};

type InlineChipProps =
  | (InlineChipCommonProps &
      Omit<
        React.ComponentPropsWithoutRef<"span">,
        keyof InlineChipCommonProps
      > & {
        as?: "span";
      })
  | (InlineChipCommonProps &
      Omit<
        React.ComponentPropsWithoutRef<"button">,
        keyof InlineChipCommonProps
      > & {
        as: "button";
      });

/**
 * Shared visual primitive for mention, channel, and Buzz permalink chips.
 *
 * Copy reads a chip's text back against the label its `data-*-label` attribute
 * declares (`matchChipTextToLabel`) to tell a whole chip from the fragment a
 * boundary-crossing selection leaves. Text rendered here that isn't part of
 * that label — a count badge, a glyph with a text fallback — makes a whole chip
 * look like a fragment, which silently drops its identity from the clipboard.
 * Render such additions as `aria-hidden` CSS content, or teach the predicate
 * the new form.
 */
export function InlineChip({
  as = "span",
  children,
  className,
  icon,
  interactive = false,
  ...props
}: InlineChipProps) {
  const classes = cn(
    MENTION_CHIP_BASE_CLASSES,
    inlineChipIconClasses(icon),
    interactive && "cursor-pointer",
    interactive && MENTION_CHIP_HOVER_CLASSES,
    className,
  );
  if (as === "button") {
    return (
      <button
        {...(props as React.ComponentPropsWithoutRef<"button">)}
        type="button"
        className={classes}
      >
        {children}
      </button>
    );
  }

  return (
    <span
      {...(props as React.ComponentPropsWithoutRef<"span">)}
      className={classes}
    >
      {children}
    </span>
  );
}

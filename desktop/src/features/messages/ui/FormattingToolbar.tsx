import * as React from "react";
import type { Editor } from "@tiptap/react";
import {
  Bold,
  Code,
  HatGlasses,
  Italic,
  Link,
  List,
  ListOrdered,
  Quote,
  SquareCode,
  Strikethrough,
} from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { getEditorSpoilerRangeState } from "@/features/messages/lib/spoilerFormatting";
import { SPOILER_MARK_NAME } from "@/features/messages/lib/spoilerMark";

type FormattingToolbarProps = {
  editor: Editor | null;
  disabled?: boolean;
  /**
   * Opens the link-edit modal for the current selection. When provided, the
   * link button routes here instead of the legacy `window.prompt` flow
   * (a no-op in the Tauri WebView).
   */
  onLinkButton?: () => void;
};

type ActiveStates = {
  bold: boolean;
  italic: boolean;
  strike: boolean;
  code: boolean;
  codeBlock: boolean;
  link: boolean;
  bulletList: boolean;
  orderedList: boolean;
  blockquote: boolean;
  spoiler: boolean;
};

function getActiveStates(editor: Editor): ActiveStates {
  return {
    bold: editor.isActive("bold"),
    italic: editor.isActive("italic"),
    strike: editor.isActive("strike"),
    code: editor.isActive("code"),
    codeBlock: editor.isActive("codeBlock"),
    link: editor.isActive("link"),
    bulletList: editor.isActive("bulletList"),
    orderedList: editor.isActive("orderedList"),
    blockquote: editor.isActive("blockquote"),
    spoiler: isSpoilerFormattingActive(editor),
  };
}

export function isSpoilerFormattingActive(editor: Editor): boolean {
  return editor.isActive(SPOILER_MARK_NAME);
}

function documentRangeForEmptySelection(editor: Editor): {
  from: number;
  to: number;
} | null {
  const { doc, selection } = editor.state;
  if (!selection.empty) return null;

  if (doc.textContent.trim().length === 0) return null;

  const from = 1;
  const to = doc.content.size - 1;
  return from < to ? { from, to } : null;
}

/**
 * Toggles the text spoiler mark. With a selection, toggles the mark on the
 * selected range; with an empty selection, applies/removes spoiler across the
 * whole document. Text-only — media spoilers are toggled per-attachment in
 * the attachment lightbox.
 */
export function toggleSpoilerFormatting(editor: Editor): void {
  const range = documentRangeForEmptySelection(editor);
  if (!range) {
    editor.chain().focus().toggleMark(SPOILER_MARK_NAME).run();
    return;
  }

  const cursorPosition = editor.state.selection.from;
  const chain = editor.chain().focus().setTextSelection(range);
  const rangeSpoilerState = getEditorSpoilerRangeState(
    editor,
    range.from,
    range.to,
  );
  if (rangeSpoilerState === "no-markable-content") {
    chain.setTextSelection(cursorPosition).run();
    return;
  }

  if (rangeSpoilerState !== "fully-spoiled") {
    chain.setMark(SPOILER_MARK_NAME).setTextSelection(cursorPosition).run();
  } else {
    chain.unsetMark(SPOILER_MARK_NAME).setTextSelection(cursorPosition).run();
  }
}

/**
 * Formatting bar shown above the editor when the format toggle is active.
 * Uses plain buttons with explicit active-class toggling instead of
 * Radix Toggle to avoid data-state / re-render issues.
 */
export const FormattingToolbar = React.memo(function FormattingToolbar({
  editor,
  disabled = false,
  onLinkButton,
}: FormattingToolbarProps) {
  const [activeStates, setActiveStates] = React.useState<ActiveStates | null>(
    () => (editor ? getActiveStates(editor) : null),
  );

  React.useEffect(() => {
    if (!editor) {
      setActiveStates(null);
      return;
    }
    setActiveStates(getActiveStates(editor));

    const onTransaction = () => {
      setActiveStates(getActiveStates(editor));
    };
    editor.on("transaction", onTransaction);
    return () => {
      editor.off("transaction", onTransaction);
    };
  }, [editor]);

  const toggleBold = React.useCallback(() => {
    editor?.chain().focus().toggleBold().run();
  }, [editor]);

  const toggleItalic = React.useCallback(() => {
    editor?.chain().focus().toggleItalic().run();
  }, [editor]);

  const toggleStrike = React.useCallback(() => {
    editor?.chain().focus().toggleStrike().run();
  }, [editor]);

  const toggleCode = React.useCallback(() => {
    editor?.chain().focus().toggleCode().run();
  }, [editor]);

  const toggleCodeBlock = React.useCallback(() => {
    editor?.chain().focus().toggleCodeBlock().run();
  }, [editor]);

  const toggleLink = React.useCallback(() => {
    if (!editor) return;

    // Preferred path: open the link-edit modal, which handles add, edit, and
    // remove with proper display-text + URL fields.
    if (onLinkButton) {
      onLinkButton();
      return;
    }

    // Legacy fallback (no modal wired): the native prompts below are a no-op
    // in the Tauri WebView, so this path effectively does nothing there.
    if (editor.isActive("link")) {
      editor.chain().focus().unsetLink().run();
      return;
    }

    const { from, to } = editor.state.selection;
    const hasSelection = from !== to;

    if (hasSelection) {
      const url = window.prompt("Enter URL:");
      if (url) {
        editor.chain().focus().setLink({ href: url }).run();
      }
    } else {
      const url = window.prompt("Enter URL:");
      if (url) {
        const label = window.prompt("Link text:", url) || url;
        editor.chain().focus().insertContent(`[${label}](${url})`).run();
      }
    }
  }, [editor, onLinkButton]);

  const toggleBulletList = React.useCallback(() => {
    editor?.chain().focus().toggleBulletList().run();
  }, [editor]);

  const toggleOrderedList = React.useCallback(() => {
    editor?.chain().focus().toggleOrderedList().run();
  }, [editor]);

  const toggleBlockquote = React.useCallback(() => {
    editor?.chain().focus().toggleBlockquote().run();
  }, [editor]);

  const toggleSpoiler = React.useCallback(() => {
    if (!editor) return;
    toggleSpoilerFormatting(editor);
  }, [editor]);

  if (!editor || !activeStates) return null;

  const items = [
    {
      icon: Bold,
      label: "Bold",
      shortcut: "⌘B",
      action: toggleBold,
      active: activeStates.bold,
    },
    {
      icon: Italic,
      label: "Italic",
      shortcut: "⌘I",
      action: toggleItalic,
      active: activeStates.italic,
    },
    {
      icon: Strikethrough,
      label: "Strikethrough",
      shortcut: "⌘⇧X",
      action: toggleStrike,
      active: activeStates.strike,
    },
    {
      icon: Code,
      label: "Code",
      shortcut: "⌘E",
      action: toggleCode,
      active: activeStates.code,
    },
    {
      icon: SquareCode,
      label: "Code block",
      action: toggleCodeBlock,
      active: activeStates.codeBlock,
    },
    {
      icon: Link,
      label: "Link",
      shortcut: "⌘K",
      action: toggleLink,
      active: activeStates.link,
    },
    {
      icon: List,
      label: "Bullet list",
      action: toggleBulletList,
      active: activeStates.bulletList,
    },
    {
      icon: ListOrdered,
      label: "Ordered list",
      action: toggleOrderedList,
      active: activeStates.orderedList,
    },
    {
      icon: Quote,
      label: "Quote",
      action: toggleBlockquote,
      active: activeStates.blockquote,
    },
    {
      icon: HatGlasses,
      label: "Spoiler",
      action: toggleSpoiler,
      active: activeStates.spoiler,
    },
  ] as const;

  return (
    <div className="flex items-center gap-0.5">
      {items.map((item) => (
        <Tooltip key={item.label} disableHoverableContent>
          <TooltipTrigger asChild>
            <button
              type="button"
              aria-label={item.label}
              aria-pressed={item.active}
              disabled={disabled}
              onClick={() => item.action()}
              className={cn(
                "inline-flex h-7 w-7 min-w-7 items-center justify-center rounded-md text-sm font-medium transition-colors",
                "hover:bg-muted hover:text-foreground",
                "focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
                "disabled:pointer-events-none disabled:opacity-50",
                "[&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
                item.active
                  ? "bg-primary text-primary-foreground"
                  : "bg-transparent text-muted-foreground",
              )}
            >
              <item.icon className="h-4 w-4" />
            </button>
          </TooltipTrigger>
          <TooltipContent>
            {"shortcut" in item
              ? `${item.label} (${item.shortcut})`
              : item.label}
          </TooltipContent>
        </Tooltip>
      ))}
    </div>
  );
});

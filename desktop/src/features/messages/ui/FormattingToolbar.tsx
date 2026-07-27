import * as React from "react";
import { TextSelection } from "@tiptap/pm/state";
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
import { isolateSelectionForBlockFormatting } from "@/features/messages/lib/selectionBlockFormatting";
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

type FormattingSelectionRange = {
  anchor: number;
  head: number;
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
  const pendingSelectionRef = React.useRef<FormattingSelectionRange | null>(
    null,
  );
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

  const captureSelection = React.useCallback(() => {
    if (!editor || editor.state.selection.empty) {
      pendingSelectionRef.current = null;
      return;
    }

    const { anchor, head } = editor.state.selection;
    pendingSelectionRef.current = { anchor, head };
  }, [editor]);

  const formattingChain = React.useCallback(() => {
    if (!editor) return null;

    const range = pendingSelectionRef.current;
    pendingSelectionRef.current = null;
    const chain = editor.chain();

    if (
      range &&
      range.anchor !== range.head &&
      range.anchor <= editor.state.doc.content.size &&
      range.head <= editor.state.doc.content.size
    ) {
      chain.command(({ tr }) => {
        tr.setSelection(TextSelection.create(tr.doc, range.anchor, range.head));
        return true;
      });
    }

    return chain.focus();
  }, [editor]);

  const toggleBold = React.useCallback(() => {
    formattingChain()?.toggleBold().run();
  }, [formattingChain]);

  const toggleItalic = React.useCallback(() => {
    formattingChain()?.toggleItalic().run();
  }, [formattingChain]);

  const toggleStrike = React.useCallback(() => {
    formattingChain()?.toggleStrike().run();
  }, [formattingChain]);

  const toggleCode = React.useCallback(() => {
    formattingChain()?.toggleCode().run();
  }, [formattingChain]);

  const toggleCodeBlock = React.useCallback(() => {
    formattingChain()
      ?.command(({ tr }) => {
        isolateSelectionForBlockFormatting(tr);
        return true;
      })
      .toggleCodeBlock()
      .run();
  }, [formattingChain]);

  const restorePendingSelection = React.useCallback(() => {
    formattingChain()?.run();
  }, [formattingChain]);

  const toggleLink = React.useCallback(() => {
    if (!editor) return;

    // Restore the range captured on pointer-down before opening the modal.
    // WKWebView may otherwise collapse it as focus moves to the toolbar.
    if (onLinkButton) {
      restorePendingSelection();
      onLinkButton();
      return;
    }

    const chain = formattingChain();
    if (!chain) return;
    chain.run();

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
  }, [editor, formattingChain, onLinkButton, restorePendingSelection]);

  const toggleBulletList = React.useCallback(() => {
    formattingChain()
      ?.command(({ tr }) => {
        isolateSelectionForBlockFormatting(tr);
        return true;
      })
      .toggleBulletList()
      .run();
  }, [formattingChain]);

  const toggleOrderedList = React.useCallback(() => {
    formattingChain()
      ?.command(({ tr }) => {
        isolateSelectionForBlockFormatting(tr);
        return true;
      })
      .toggleOrderedList()
      .run();
  }, [formattingChain]);

  const toggleBlockquote = React.useCallback(() => {
    formattingChain()?.toggleBlockquote().run();
  }, [formattingChain]);

  const toggleSpoiler = React.useCallback(() => {
    if (!editor) return;
    restorePendingSelection();
    toggleSpoilerFormatting(editor);
  }, [editor, restorePendingSelection]);

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
              onMouseDown={captureSelection}
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

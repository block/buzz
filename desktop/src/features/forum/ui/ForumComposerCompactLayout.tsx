import type * as React from "react";

import { EditorContent, type Editor } from "@tiptap/react";
import { Plus } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";

type ForumComposerCompactLayoutProps = {
  editor: Editor | null;
  header?: React.ReactNode;
  isSending?: boolean;
  onEditorKeyDown: React.KeyboardEventHandler<HTMLDivElement>;
  sendDisabled?: boolean;
};

export function ForumComposerCompactLayout({
  editor,
  header,
  isSending,
  onEditorKeyDown,
  sendDisabled,
}: ForumComposerCompactLayoutProps) {
  return (
    <div className="flex min-h-10 items-center gap-3">
      {header ? (
        <div className="flex min-w-0 shrink-0 items-center">{header}</div>
      ) : null}
      {/* biome-ignore lint/a11y/noStaticElementInteractions: keydown handler bridges Tiptap editor to autocomplete and submit */}
      <div
        className="rich-text-composer max-h-10 min-w-0 flex-1 overflow-y-auto"
        onKeyDown={onEditorKeyDown}
      >
        <EditorContent editor={editor} />
      </div>
      <Button
        aria-label={isSending ? "Sending" : "Send message"}
        className={cn(
          "h-7 w-7 shrink-0 rounded-full border border-border/70 bg-transparent p-0 text-muted-foreground shadow-none hover:bg-transparent hover:text-foreground",
        )}
        data-testid="send-message"
        disabled={sendDisabled || isSending}
        size="icon"
        type="submit"
        variant="ghost"
      >
        {isSending ? (
          <Spinner
            aria-hidden
            className="h-4 w-4 border-2 text-primary-foreground"
          />
        ) : (
          <Plus aria-hidden className="h-4 w-4" />
        )}
      </Button>
    </div>
  );
}

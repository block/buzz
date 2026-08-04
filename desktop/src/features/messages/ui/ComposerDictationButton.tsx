import * as React from "react";
import type { Editor } from "@tiptap/react";
import { Mic } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { buildDictationInsert } from "@/features/messages/lib/dictationInsert";
import { useDictation } from "@/features/messages/lib/useDictation";

/**
 * Mic button for the composer toolbar.
 *
 * Self-contained on purpose: it owns the dictation session and writes
 * transcripts straight into the editor it is handed. Both `MessageComposer`
 * and `ForumComposer` render the shared toolbar, so neither needed changes to
 * gain dictation.
 *
 * Rendered only when the `dictation` preview flag is on — the caller gates it,
 * so the model-availability check and mic teardown never run for users who
 * have not opted in.
 */
export function ComposerDictationButton({
  disabled,
  editor,
  onMouseDown,
}: {
  disabled: boolean;
  editor: Editor | null;
  onMouseDown: () => void;
}) {
  const editorRef = React.useRef(editor);
  editorRef.current = editor;

  const handleTranscript = React.useCallback((text: string) => {
    const activeEditor = editorRef.current;
    if (!activeEditor) return;
    // Text before the caret decides whether this segment needs a leading
    // space. `textBetween` over the document start → caret keeps the check
    // correct across paragraph breaks.
    const { from } = activeEditor.state.selection;
    const preceding = activeEditor.state.doc.textBetween(0, from, "\n", "\n");
    const insert = buildDictationInsert(preceding, text);
    if (!insert) return;
    activeEditor.chain().focus().insertContent(insert).run();
  }, []);

  const dictation = useDictation(handleTranscript, disabled);

  const isRecording = dictation.status === "recording";
  const isBusy = dictation.status === "starting";

  const label = !dictation.isAvailable
    ? "Speech model is still downloading"
    : dictation.status === "error"
      ? (dictation.error ?? "Dictation failed")
      : isRecording
        ? "Stop dictation"
        : "Dictate a message";

  return (
    <Tooltip disableHoverableContent>
      <TooltipTrigger asChild>
        <Button
          aria-label={isRecording ? "Stop dictation" : "Dictate a message"}
          aria-pressed={isRecording}
          data-testid="message-dictate"
          disabled={disabled || !dictation.isAvailable}
          onClick={dictation.toggle}
          onMouseDown={onMouseDown}
          size="icon"
          type="button"
          variant={isRecording ? "default" : "ghost"}
        >
          <Mic
            className={isRecording || isBusy ? "animate-pulse" : undefined}
          />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

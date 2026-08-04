import * as React from "react";

import {
  FRESH_DRAFT_KEY,
  loadComposerDraft,
  saveComposerDraft,
  saveComposerDraftIfEmpty,
} from "@/features/dev-mode/lib/composerDrafts";

/**
 * Per-channel composer drafts: restore the scope's stashed text on entry and
 * stash the current text on exit, so switching channels never carries a
 * half-typed message along. The shell must call this after its
 * channel-switch reset effect so the restore's setInput lands after
 * stopEditing's pre-edit put-back when a switch abandons an in-flight edit.
 */
export function useComposerDrafts({
  view,
  channelId,
  input,
  setInput,
  peekPreEditInput,
}: {
  view: "fresh" | "navigator" | "channel";
  /** The open channel — null while fresh/navigator or when it vanished. */
  channelId: string | null;
  input: string;
  setInput: React.Dispatch<React.SetStateAction<string>>;
  /** The stashed pre-edit draft while `e`-editing a message, else null. */
  peekPreEditInput: () => string | null;
}): {
  draftKey: string;
  restoreFailedPrompt: (originDraftKey: string, prompt: string) => void;
} {
  const draftKey =
    view === "channel" && channelId ? channelId : FRESH_DRAFT_KEY;

  const inputRef = React.useRef(input);
  inputRef.current = input;
  const draftKeyRef = React.useRef(draftKey);
  draftKeyRef.current = draftKey;
  const peekPreEditInputRef = React.useRef(peekPreEditInput);
  peekPreEditInputRef.current = peekPreEditInput;

  // biome-ignore lint/correctness/useExhaustiveDependencies: draftKey is the sole trigger
  React.useEffect(() => {
    setInput(loadComposerDraft(draftKey));
    return () => {
      // Leaving mid-edit abandons the edit like Escape does: the scope keeps
      // its pre-edit draft, not the edit buffer.
      saveComposerDraft(
        draftKey,
        peekPreEditInputRef.current() ?? inputRef.current,
      );
    };
  }, [draftKey]);

  // A failed send puts the prompt back where it came from: into the box when
  // the user is still there (unless they typed on), otherwise into that
  // scope's draft slot.
  const restoreFailedPrompt = React.useCallback(
    (originDraftKey: string, prompt: string) => {
      if (draftKeyRef.current === originDraftKey) {
        setInput((current) => (current === "" ? prompt : current));
      } else {
        saveComposerDraftIfEmpty(originDraftKey, prompt);
      }
    },
    [setInput],
  );

  return { draftKey, restoreFailedPrompt };
}

import * as React from "react";

import { openUrl } from "@tauri-apps/plugin-opener";

import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  Popover,
  PopoverAnchor,
  PopoverContent,
} from "@/shared/ui/popover";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

import type {
  LinkSelectionInfo,
  UseRichTextEditorResult,
} from "./useRichTextEditor";

type DraftState = {
  text: string;
  url: string;
  from: number;
  to: number;
  /** Whether the targeted range already carried a link (enables Remove). */
  isExistingLink: boolean;
};

type PopoverState = {
  text: string;
  url: string;
  from: number;
  to: number;
  /** Viewport rect of the clicked link, used to anchor the popover. */
  rect: DOMRect;
};

/**
 * Owns the link UX for a composer: an info popover shown when a set link is
 * clicked, plus the add/edit modal. Replaces the old `window.prompt` flow (a
 * no-op in the Tauri WebView).
 *
 * Clicking a set link surfaces an info-only popover (display text + URL, the
 * URL a real hyperlink that opens the link) with Edit and Remove — so a user
 * can tweak display text inline without a takeover modal. The modal is reached
 * via the popover's Edit button (focus on display text) and the toolbar's Add
 * flow (focus on URL).
 *
 * Returns:
 * - `openFromToolbar` — wire to the formatting toolbar's link button. Opens
 *   the modal seeded from the current selection (existing link or selected
 *   text).
 * - `openFromClick` — wire to `useRichTextEditor`'s `onEditLink`. Opens the
 *   info popover anchored at the clicked link.
 * - `dialog` — render once inside the composer tree (popover + modal).
 */
export function useLinkEditor(richText: UseRichTextEditorResult) {
  const { getLinkSelectionInfo, applyLink, removeLink } = richText;
  const [draft, setDraft] = React.useState<DraftState | null>(null);
  const [popover, setPopover] = React.useState<PopoverState | null>(null);
  const textId = React.useId();
  const urlId = React.useId();

  // Clicking a set link → info popover anchored at the clicked link.
  const openFromClick = React.useCallback(
    (info: LinkSelectionInfo, rect: DOMRect) => {
      setPopover({
        text: info.text,
        url: info.href,
        from: info.from,
        to: info.to,
        rect,
      });
    },
    [],
  );

  const closePopover = React.useCallback(() => setPopover(null), []);

  // Opens the modal seeded from a link's range. `focusUrl` decides which input
  // takes initial focus (URL for Add, display text for Edit).
  const openModal = React.useCallback((state: DraftState) => {
    setDraft(state);
  }, []);

  const openFromToolbar = React.useCallback(() => {
    const info = getLinkSelectionInfo();
    if (info) {
      openModal({
        text: info.text,
        url: info.href,
        from: info.from,
        to: info.to,
        isExistingLink: info.href.length > 0,
      });
      return;
    }
    // No selection and no link under the caret — open an empty modal that
    // inserts a fresh link at the caret on save.
    openModal({ text: "", url: "", from: 0, to: 0, isExistingLink: false });
  }, [getLinkSelectionInfo, openModal]);

  // Popover Edit → close the popover, open the modal on the same range.
  const editFromPopover = React.useCallback(() => {
    if (!popover) return;
    openModal({
      text: popover.text,
      url: popover.url,
      from: popover.from,
      to: popover.to,
      isExistingLink: true,
    });
    closePopover();
  }, [popover, openModal, closePopover]);

  const removeFromPopover = React.useCallback(() => {
    if (!popover) return;
    removeLink({ from: popover.from, to: popover.to });
    closePopover();
  }, [popover, removeLink, closePopover]);

  const close = React.useCallback(() => setDraft(null), []);

  const save = React.useCallback(() => {
    if (!draft) return;
    const url = draft.url.trim();
    if (!url) return;
    if (draft.from === 0 && draft.to === 0) {
      // Empty-caret insert: fall back to current selection range.
      const info = getLinkSelectionInfo();
      applyLink({
        href: url,
        text: draft.text,
        from: info?.from ?? 0,
        to: info?.to ?? 0,
      });
    } else {
      applyLink({ href: url, text: draft.text, from: draft.from, to: draft.to });
    }
    close();
  }, [draft, applyLink, getLinkSelectionInfo, close]);

  const remove = React.useCallback(() => {
    if (!draft) return;
    removeLink({ from: draft.from, to: draft.to });
    close();
  }, [draft, removeLink, close]);

  // Add link (no existing link) focuses the URL; Edit focuses display text.
  const focusUrlFirst = draft ? !draft.isExistingLink : false;

  const popoverCard = (
    <Popover
      open={popover !== null}
      onOpenChange={(open) => {
        if (!open) closePopover();
      }}
    >
      <PopoverAnchor
        style={{
          position: "fixed",
          left: popover?.rect.left ?? 0,
          top: popover?.rect.top ?? 0,
          width: 0,
          height: popover?.rect.height ?? 0,
        }}
      />
      <PopoverContent
        align="start"
        side="top"
        className="flex w-72 flex-col gap-2"
        // Keep editor focus on close — the caret already sits where the user
        // clicked, so we don't want Radix to refocus the trigger.
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        <div className="text-sm font-medium break-words">
          {popover?.text}
        </div>
        <a
          href={popover?.url}
          className="text-primary text-xs underline underline-offset-4 break-all"
          onClick={(event) => {
            event.preventDefault();
            if (popover?.url) void openUrl(popover.url);
          }}
        >
          {popover?.url}
        </a>
        <div className="mt-1 flex items-center justify-end gap-2">
          <Button
            type="button"
            variant="destructive"
            size="sm"
            onClick={removeFromPopover}
          >
            Remove
          </Button>
          <Button
            type="button"
            size="sm"
            onClick={editFromPopover}
          >
            Edit
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );

  const dialog = (
    <Dialog
      open={draft !== null}
      onOpenChange={(open) => {
        if (!open) close();
      }}
    >
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            {draft?.isExistingLink ? "Edit link" : "Add link"}
          </DialogTitle>
        </DialogHeader>
        <form
          className="flex flex-col gap-3"
          onSubmit={(event) => {
            event.preventDefault();
            save();
          }}
        >
          <label
            className="flex flex-col gap-1 text-sm font-medium"
            htmlFor={textId}
          >
            Display text
            <Input
              id={textId}
              autoFocus={!focusUrlFirst}
              placeholder="Text to display"
              value={draft?.text ?? ""}
              onChange={(event) =>
                setDraft((prev) =>
                  prev ? { ...prev, text: event.target.value } : prev,
                )
              }
            />
          </label>
          <label
            className="flex flex-col gap-1 text-sm font-medium"
            htmlFor={urlId}
          >
            URL
            <Input
              id={urlId}
              autoFocus={focusUrlFirst}
              placeholder="https://example.com"
              value={draft?.url ?? ""}
              onChange={(event) =>
                setDraft((prev) =>
                  prev ? { ...prev, url: event.target.value } : prev,
                )
              }
            />
          </label>
          <div className="mt-2 flex items-center justify-between gap-2">
            {draft?.isExistingLink ? (
              <Button type="button" variant="destructive" onClick={remove}>
                Remove
              </Button>
            ) : (
              <span />
            )}
            <div className="flex items-center gap-2">
              <Button type="button" variant="ghost" onClick={close}>
                Cancel
              </Button>
              <Button type="submit" disabled={!draft?.url.trim()}>
                Save
              </Button>
            </div>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );

  return {
    openFromToolbar,
    openFromClick,
    dialog: (
      <>
        {popoverCard}
        {dialog}
      </>
    ),
  };
}

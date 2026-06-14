import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Pencil, Unlink } from "lucide-react";

import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Popover, PopoverAnchor, PopoverContent } from "@/shared/ui/popover";

import type {
  LinkSelectionInfo,
  UseRichTextEditorResult,
} from "./useRichTextEditor";
import {
  getLinkEditorInitialFocus,
  type LinkEditorInitialFocus,
} from "./linkEditorFocus";

type DraftState = {
  text: string;
  url: string;
  from: number;
  to: number;
  /** Whether the targeted range already carried a link (enables Remove). */
  isExistingLink: boolean;
  initialFocus: LinkEditorInitialFocus;
};

type LinkCardState = {
  info: LinkSelectionInfo;
  rect: {
    left: number;
    top: number;
    width: number;
    height: number;
  };
};

/**
 * Owns composer link controls. Existing links show an anchored hover card on
 * click/caret movement; the card can remove the link, open its URL, or enter
 * the existing shadcn dialog to edit display text and URL. Toolbar-created
 * links still open the dialog directly.
 *
 * Returns:
 * - `openFromToolbar` — wire to the formatting toolbar's link button. Seeds
 *   the dialog from the current selection (existing link or selected text).
 * - `openFromClick` — wire to `useRichTextEditor`'s `onEditLink`. Moves the
 *   clicked link into the hover-card state.
 * - `showFromCursor` — wire to cursor/selection updates to show the same card
 *   when arrow-key movement lands inside a link.
 * - `card`/`dialog` — render once inside the composer tree.
 */
export function useLinkEditor(richText: UseRichTextEditorResult) {
  const { getLinkSelectionInfo, applyLink, removeLink } = richText;
  const [draft, setDraft] = React.useState<DraftState | null>(null);
  const [cardState, setCardState] = React.useState<LinkCardState | null>(null);
  const cardContentRef = React.useRef<HTMLDivElement>(null);
  const textId = React.useId();
  const urlId = React.useId();

  const getLinkRect = React.useCallback(
    (info: LinkSelectionInfo): LinkCardState["rect"] | null => {
      const editor = richText.editor;
      if (!editor) return null;

      try {
        const range = document.createRange();
        const start = editor.view.domAtPos(info.from);
        const end = editor.view.domAtPos(info.to);
        range.setStart(start.node, start.offset);
        range.setEnd(end.node, end.offset);

        const rect = range.getBoundingClientRect();
        range.detach();
        if (rect.width > 0 || rect.height > 0) {
          return {
            left: rect.left,
            top: rect.top,
            width: rect.width,
            height: rect.height,
          };
        }
      } catch {
        // Fall through to the caret coordinates below.
      }

      const coords = editor.view.coordsAtPos(info.from);
      return {
        left: coords.left,
        top: coords.top,
        width: Math.max(1, coords.right - coords.left),
        height: Math.max(1, coords.bottom - coords.top),
      };
    },
    [richText.editor],
  );

  const showCard = React.useCallback(
    (info: LinkSelectionInfo | null) => {
      if (!info) {
        setCardState(null);
        return;
      }

      const rect = getLinkRect(info);
      if (!rect) {
        setCardState(null);
        return;
      }

      setCardState((prev) => {
        const sameLink =
          prev?.info.href === info.href &&
          prev.info.from === info.from &&
          prev.info.to === info.to &&
          prev.info.text === info.text;
        const sameRect =
          prev?.rect.left === rect.left &&
          prev.rect.top === rect.top &&
          prev.rect.width === rect.width &&
          prev.rect.height === rect.height;
        if (sameLink && sameRect) return prev;
        return { info, rect };
      });
    },
    [getLinkRect],
  );

  const openDialogFromInfo = React.useCallback((info: LinkSelectionInfo) => {
    setDraft({
      text: info.text,
      url: info.href,
      from: info.from,
      to: info.to,
      isExistingLink: info.href.length > 0,
      initialFocus: getLinkEditorInitialFocus(info),
    });
  }, []);

  const openFromClick = React.useCallback(
    (info: LinkSelectionInfo) => {
      showCard(info);
    },
    [showCard],
  );

  const openFromToolbar = React.useCallback(() => {
    const info = getLinkSelectionInfo();
    if (info) {
      openDialogFromInfo(info);
      return;
    }
    // No selection and no link under the caret — open an empty modal that
    // inserts a fresh link at the caret on save.
    setDraft({
      text: "",
      url: "",
      from: 0,
      to: 0,
      isExistingLink: false,
      initialFocus: "text",
    });
  }, [getLinkSelectionInfo, openDialogFromInfo]);

  const close = React.useCallback(() => setDraft(null), []);

  const closeCard = React.useCallback(() => setCardState(null), []);

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
      applyLink({
        href: url,
        text: draft.text,
        from: draft.from,
        to: draft.to,
      });
    }
    close();
  }, [draft, applyLink, getLinkSelectionInfo, close]);

  const remove = React.useCallback(() => {
    if (!draft) return;
    removeLink({ from: draft.from, to: draft.to });
    close();
  }, [draft, removeLink, close]);

  const removeFromCard = React.useCallback(() => {
    if (!cardState) return;
    removeLink({ from: cardState.info.from, to: cardState.info.to });
    closeCard();
  }, [cardState, removeLink, closeCard]);

  const editFromCard = React.useCallback(() => {
    if (!cardState) return;
    openDialogFromInfo(cardState.info);
    closeCard();
  }, [cardState, openDialogFromInfo, closeCard]);

  const openCardUrl = React.useCallback(
    (event: React.MouseEvent<HTMLAnchorElement>) => {
      const href = cardState?.info.href.trim();
      if (!href) return;
      event.preventDefault();
      void openUrl(href);
    },
    [cardState],
  );

  const refreshCardRect = React.useCallback(() => {
    setCardState((prev) => {
      if (!prev) return prev;
      const rect = getLinkRect(prev.info);
      return rect ? { ...prev, rect } : null;
    });
  }, [getLinkRect]);

  React.useEffect(() => {
    if (!cardState) return;

    window.addEventListener("resize", refreshCardRect);
    window.addEventListener("scroll", refreshCardRect, true);

    return () => {
      window.removeEventListener("resize", refreshCardRect);
      window.removeEventListener("scroll", refreshCardRect, true);
    };
  }, [cardState, refreshCardRect]);

  const focusCardFirstControl = React.useCallback((): boolean => {
    if (!cardState) return false;
    const target = cardContentRef.current?.querySelector<HTMLElement>(
      "a[href], button:not([disabled])",
    );
    if (!target) return false;
    target.focus();
    return true;
  }, [cardState]);

  const card = (
    <Popover
      open={cardState !== null}
      onOpenChange={(open) => {
        if (!open) closeCard();
      }}
    >
      {cardState ? (
        <PopoverAnchor asChild>
          <span
            aria-hidden="true"
            style={{
              height: cardState.rect.height,
              left: cardState.rect.left,
              pointerEvents: "none",
              position: "fixed",
              top: cardState.rect.top,
              width: cardState.rect.width,
            }}
          />
        </PopoverAnchor>
      ) : null}
      {cardState ? (
        <PopoverContent
          align="center"
          className="w-80 rounded-xl p-3"
          onCloseAutoFocus={(event) => event.preventDefault()}
          onInteractOutside={(event) => {
            const target = event.detail.originalEvent.target;
            if (
              target instanceof Element &&
              target.closest(".rich-text-composer a[href]")
            ) {
              event.preventDefault();
            }
          }}
          onOpenAutoFocus={(event) => event.preventDefault()}
          ref={cardContentRef}
          side="top"
          sideOffset={8}
        >
          <div className="flex flex-col gap-3">
            <div className="min-w-0">
              <p className="mb-1 text-[11px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
                Display text
              </p>
              <p className="truncate text-sm font-medium text-popover-foreground">
                {cardState.info.text || cardState.info.href}
              </p>
            </div>
            <a
              className="break-all text-sm text-primary underline underline-offset-4 outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
              href={cardState.info.href}
              onClick={openCardUrl}
              rel="noreferrer noopener"
              target="_blank"
            >
              {cardState.info.href}
            </a>
            <div className="flex items-center gap-2">
              <Button
                aria-label="Unlink"
                className="h-8 w-8 shrink-0 text-primary hover:text-primary"
                onClick={removeFromCard}
                size="icon"
                type="button"
                variant="ghost"
              >
                <Unlink className="h-4 w-4" />
              </Button>
              <Button
                className="h-8 px-3"
                onClick={editFromCard}
                size="sm"
                type="button"
                variant="outline"
              >
                <Pencil className="h-3.5 w-3.5" />
                Edit
              </Button>
            </div>
          </div>
        </PopoverContent>
      ) : null}
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
              autoFocus={draft?.initialFocus === "text"}
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
              autoFocus={draft?.initialFocus === "url"}
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
    showFromCursor: showCard,
    focusCardFirstControl,
    isCardOpen: cardState !== null,
    card,
    dialog,
  };
}

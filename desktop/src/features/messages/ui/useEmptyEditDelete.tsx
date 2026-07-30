import * as React from "react";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";

import { resolveEmptyEditDelete } from "./emptyEditDelete";

type EmptyEditDeleteParams = {
  /** Live ref to the message currently loaded into the composer for editing. */
  editTargetRef: React.RefObject<{ id: string } | null | undefined>;
  /**
   * Owner handler that deletes the edited message and exits edit mode. When
   * undefined, clearing an edit to empty stays an inert no-op.
   */
  onDeleteEditTarget?: (eventId: string) => void | Promise<void>;
  /** Empties the composer body so it doesn't linger after the delete. */
  clearComposerBody: () => void;
};

/**
 * Turns "clear an edit to empty and submit" into the delete-message action.
 *
 * Deleting a message by right-click → "Delete message" pops a confirmation
 * before anything is destroyed; this mirrors that exactly. `requestDelete`
 * (called from the composer's submit guard when the edited body is empty and
 * has no attachments) opens the same confirmation, and only on confirm does it
 * clear the composer and hand the target id to the owner's delete handler. A
 * confirmation — rather than an instant delete — means an accidental
 * clear-and-Enter can't destroy a message with no undo.
 *
 * Returns the trigger plus the dialog element to render inside the composer.
 * No-ops (and never opens the dialog) when no delete handler is wired.
 */
export function useEmptyEditDelete({
  editTargetRef,
  onDeleteEditTarget,
  clearComposerBody,
}: EmptyEditDeleteParams) {
  const [isOpen, setIsOpen] = React.useState(false);

  // Stash the owner handler + clear callback in refs so the returned callbacks
  // stay reference-stable across renders (the composer feeds them into large
  // dependency arrays and memoized children).
  const onDeleteEditTargetRef = React.useRef(onDeleteEditTarget);
  onDeleteEditTargetRef.current = onDeleteEditTarget;
  const clearComposerBodyRef = React.useRef(clearComposerBody);
  clearComposerBodyRef.current = clearComposerBody;

  const requestDelete = React.useCallback(() => {
    // No target / no delete handler → keep the historical no-op and never
    // surface the dialog.
    const eventId = resolveEmptyEditDelete(
      editTargetRef.current?.id,
      Boolean(onDeleteEditTargetRef.current),
    );
    if (eventId !== null) {
      setIsOpen(true);
    }
  }, [editTargetRef]);

  const confirmDelete = React.useCallback(() => {
    setIsOpen(false);
    // Re-resolve at confirm time. The AlertDialog is modal, so the edit target
    // can't change underneath it; if it somehow cleared, the null id no-ops.
    const eventId = resolveEmptyEditDelete(
      editTargetRef.current?.id,
      Boolean(onDeleteEditTargetRef.current),
    );
    if (eventId === null) {
      return;
    }
    clearComposerBodyRef.current();
    void onDeleteEditTargetRef.current?.(eventId);
  }, [editTargetRef]);

  const dialog = (
    <AlertDialog onOpenChange={setIsOpen} open={isOpen}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete message?</AlertDialogTitle>
          <AlertDialogDescription>
            This will permanently delete this message and cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button type="button" variant="outline">
              Cancel
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction asChild>
            <Button
              data-testid="confirm-empty-edit-delete"
              onClick={confirmDelete}
              type="button"
              variant="destructive"
            >
              Delete
            </Button>
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );

  return {
    requestEmptyEditDelete: requestDelete,
    emptyEditDeleteDialog: dialog,
  };
}

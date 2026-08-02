import * as React from "react";
import { useTranslation } from "react-i18next";

import {
  AgentDefaultsEditor,
  type GlobalAgentConfigSaveResult,
} from "./AgentDefaultsEditor";
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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

export function AgentDefaultsDialog({
  open,
  onOpenChange,
  returnFocusRef,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  returnFocusRef: React.RefObject<HTMLButtonElement | null>;
}) {
  const { t } = useTranslation();
  const [dirty, setDirty] = React.useState(false);
  const [saving, setSaving] = React.useState(false);
  const [confirmDiscard, setConfirmDiscard] = React.useState(false);
  const [restartFailures, setRestartFailures] = React.useState(0);
  const secondaryActionRef = React.useRef<HTMLButtonElement>(null);
  const restoreDefaultsFocusRef = React.useRef(false);

  const closeAndRestoreFocus = React.useCallback(() => {
    onOpenChange(false);
    requestAnimationFrame(() => returnFocusRef.current?.focus());
  }, [onOpenChange, returnFocusRef]);

  const requestClose = React.useCallback(() => {
    if (saving) return;
    if (dirty) {
      setConfirmDiscard(true);
      return;
    }
    closeAndRestoreFocus();
  }, [closeAndRestoreFocus, dirty, saving]);

  function handleSaveSuccess(result: GlobalAgentConfigSaveResult) {
    if (result.failed_restart_count > 0) {
      setRestartFailures(result.failed_restart_count);
      return;
    }
    closeAndRestoreFocus();
  }

  React.useEffect(() => {
    if (open) {
      setDirty(false);
      setConfirmDiscard(false);
      setRestartFailures(0);
    }
  }, [open]);

  return (
    <>
      <Dialog
        open={open}
        onOpenChange={(nextOpen) => {
          if (!nextOpen) requestClose();
        }}
      >
        <DialogContent
          className="max-h-[calc(100vh-2rem)] max-w-2xl overflow-y-auto"
          data-testid="agent-ai-defaults-dialog"
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            returnFocusRef.current?.focus();
          }}
        >
          <DialogHeader>
            <DialogTitle>{t("agents.defaults")}</DialogTitle>
            <DialogDescription>
              {t("agents.defaultsDialogDescription")}
            </DialogDescription>
          </DialogHeader>
          <AgentDefaultsEditor
            layout="flat"
            onDirtyChange={setDirty}
            onSaveSuccess={handleSaveSuccess}
            onSavingChange={setSaving}
            secondaryAction={
              <Button
                disabled={saving}
                ref={secondaryActionRef}
                onClick={
                  restartFailures > 0 ? closeAndRestoreFocus : requestClose
                }
                size="sm"
                type="button"
                variant="outline"
              >
                {restartFailures > 0 ? t("common.done") : t("common.cancel")}
              </Button>
            }
          />
        </DialogContent>
      </Dialog>

      <AlertDialog open={confirmDiscard} onOpenChange={setConfirmDiscard}>
        <AlertDialogContent
          data-testid="discard-ai-defaults-dialog"
          onCloseAutoFocus={(event) => {
            if (!restoreDefaultsFocusRef.current) return;
            event.preventDefault();
            restoreDefaultsFocusRef.current = false;
            requestAnimationFrame(() => secondaryActionRef.current?.focus());
          }}
        >
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("agents.discardDefaultsTitle")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("agents.discardDefaultsDescription")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel
              onClick={() => {
                restoreDefaultsFocusRef.current = true;
              }}
            >
              {t("common.keepEditing")}
            </AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button
                onClick={() => {
                  setConfirmDiscard(false);
                  closeAndRestoreFocus();
                }}
                variant="destructive"
              >
                {t("common.discardChanges")}
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

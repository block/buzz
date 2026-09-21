import { useTranslation } from "@/i18n";
import { Button } from "@/shared/ui/button";

type AgentDefinitionDialogFooterProps = {
  canSubmit: boolean;
  isAvatarUploadPending: boolean;
  isPending: boolean;
  onCancel: () => void;
  publishesCatalogUpdates: boolean;
  submitLabel: string;
};

export function AgentDefinitionDialogFooter({
  canSubmit,
  isAvatarUploadPending,
  isPending,
  onCancel,
  publishesCatalogUpdates,
  submitLabel,
}: AgentDefinitionDialogFooterProps) {
  const { t } = useTranslation();
  return (
    <div className="flex w-full flex-wrap items-center justify-between gap-3">
      <div className="flex min-h-9 min-w-0 flex-wrap items-center gap-3">
        {publishesCatalogUpdates ? (
          <p
            className="max-w-sm text-xs text-muted-foreground"
            data-testid="persona-dialog-catalog-publish-notice"
          >
            {t("agents.definition-footer.catalog-notice")}
          </p>
        ) : null}
      </div>

      <div className="flex items-center gap-2">
        <Button
          disabled={isPending || isAvatarUploadPending}
          onClick={onCancel}
          type="button"
          variant="outline"
        >
          {t("agents.team-dialog.cancel")}
        </Button>
        <Button
          data-testid="persona-dialog-submit"
          disabled={!canSubmit}
          form="persona-dialog-form"
          type="submit"
        >
          {isPending
            ? t("agents.team-dialog.saving")
            : isAvatarUploadPending
              ? t("agents.definition-footer.uploading")
              : publishesCatalogUpdates
                ? t("agents.definition-footer.save-and-publish")
                : submitLabel}
        </Button>
      </div>
    </div>
  );
}

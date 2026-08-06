import { FolderOpen, TriangleAlert } from "lucide-react";

import { useVaultLifecycle } from "@/features/documents/useVaultLifecycle";
import {
  setAlwaysLivePreview,
  useAlwaysLivePreview,
} from "@/features/documents/useDocumentsPreferences";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { Switch } from "@/shared/ui/switch";

/**
 * Settings → Documents. Gated on the `documents` preview feature.
 *
 * The vault is stored per-machine rather than per-community: notes belong to
 * the person, not to a relay, so switching communities must not swap them.
 */
export function DocumentsSettingsCard() {
  const { activation, chooseVault, forgetVault, vaultPath } =
    useVaultLifecycle();
  const alwaysLivePreview = useAlwaysLivePreview();

  return (
    <section className="space-y-4" data-testid="settings-documents">
      <div>
        <h2 className="text-sm font-medium">Documents</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Read and edit markdown files from a folder on this computer. The vault
          is stored per-machine — it does not change when you switch
          communities.
        </p>
      </div>

      <Card className="p-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="text-sm font-medium">Vault folder</p>
            {vaultPath ? (
              <p
                className="mt-1 break-all text-sm text-muted-foreground"
                data-testid="documents-vault-path"
              >
                {vaultPath}
              </p>
            ) : (
              <p className="mt-1 text-sm text-muted-foreground">
                No folder selected yet.
              </p>
            )}
          </div>

          <div className="flex shrink-0 items-center gap-2">
            {vaultPath ? (
              <Button
                data-testid="documents-settings-forget-vault"
                onClick={forgetVault}
                size="sm"
                type="button"
                variant="ghost"
              >
                Forget
              </Button>
            ) : null}
            <Button
              data-testid="documents-settings-choose-vault"
              onClick={() => void chooseVault()}
              size="sm"
              type="button"
              variant={vaultPath ? "outline" : "default"}
            >
              <FolderOpen className="h-4 w-4" />
              {vaultPath ? "Change folder" : "Choose folder"}
            </Button>
          </div>
        </div>

        {activation.status === "error" ? (
          <p
            className="mt-3 flex items-start gap-2 rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive"
            data-testid="documents-settings-vault-error"
          >
            <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
            <span>{activation.message}</span>
          </p>
        ) : null}
      </Card>

      <Card className="p-4">
        <div className="flex items-start justify-between gap-4">
          <label className="min-w-0" htmlFor="documents-always-live-preview">
            <p className="text-sm font-medium">Always open in live preview</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Some markdown cannot be represented in the editor — callouts,
              footnotes and raw HTML. Those notes normally open in source mode
              so editing them cannot rewrite the file. Turn this on to open
              everything in live preview instead; saving such a note will
              reformat it.
            </p>
          </label>
          <Switch
            checked={alwaysLivePreview}
            data-testid="documents-always-live-preview"
            id="documents-always-live-preview"
            onCheckedChange={setAlwaysLivePreview}
          />
        </div>
      </Card>
    </section>
  );
}

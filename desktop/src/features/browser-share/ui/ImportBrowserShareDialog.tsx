import * as React from "react";
import { toast } from "sonner";

import type { PlaygroundCard } from "@/features/playground/lib/types";
import { usePinnedSites } from "@/features/pinned-sites/hooks";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Textarea } from "@/shared/ui/textarea";

import {
  applyBrowserShareAsSession,
  prepareBrowserShareAsPin,
} from "../lib/applyImport";
import { parseBrowserShareJson } from "../lib/serialize";
import {
  BROWSER_SHARE_SECURITY_NOTE,
  type HulaBrowserShareV1,
} from "../lib/types";

export type ImportBrowserShareTarget = "session" | "pin";

export function ImportBrowserShareDialog({
  open,
  onOpenChange,
  onImportedSession,
  allowPin = true,
  defaultTarget = "session",
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onImportedSession?: (card: PlaygroundCard) => void;
  allowPin?: boolean;
  defaultTarget?: ImportBrowserShareTarget;
}) {
  const { savePin } = usePinnedSites();
  const [raw, setRaw] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [target, setTarget] =
    React.useState<ImportBrowserShareTarget>(defaultTarget);
  const [busy, setBusy] = React.useState(false);
  const fileInputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (!open) return;
    setRaw("");
    setError(null);
    setTarget(defaultTarget);
    setBusy(false);
  }, [defaultTarget, open]);

  function readShare(): HulaBrowserShareV1 | null {
    const share = parseBrowserShareJson(raw);
    if (!share) {
      setError("Paste a valid hula-browser-share.v1 JSON file.");
      return null;
    }
    setError(null);
    return share;
  }

  async function handleImport() {
    const share = readShare();
    if (!share) return;
    setBusy(true);
    try {
      if (target === "session") {
        const applied = applyBrowserShareAsSession(share);
        if (!applied) {
          setError("Could not open that URL as a browser.");
          return;
        }
        onImportedSession?.(applied.card);
        toast.success("Browser imported with runbook");
        onOpenChange(false);
        return;
      }
      const prepared = prepareBrowserShareAsPin(share);
      if (!prepared) {
        setError("Could not save that URL as a pin.");
        return;
      }
      const pin = await savePin({ draft: prepared.draft });
      prepared.writeRunbookForPinId(pin.id);
      toast.success("Pinned site imported with runbook");
      onOpenChange(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Import failed.");
    } finally {
      setBusy(false);
    }
  }

  function onFileSelected(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => {
      const text = typeof reader.result === "string" ? reader.result : "";
      setRaw(text);
      setError(null);
    };
    reader.onerror = () => setError("Could not read that file.");
    reader.readAsText(file);
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <ChooserDialogContent
        footer={
          <div className="flex justify-end gap-2">
            <Button
              onClick={() => onOpenChange(false)}
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
            <Button
              data-testid="browser-share-import-submit"
              disabled={busy || !raw.trim()}
              onClick={() => void handleImport()}
              type="button"
            >
              Import
            </Button>
          </div>
        }
        title="Add browser from share"
      >
        <div className="space-y-4">
          <p className="text-sm text-muted-foreground">
            {BROWSER_SHARE_SECURITY_NOTE}
          </p>
          <div className="flex flex-wrap gap-2">
            <Button
              data-testid="browser-share-import-pick-file"
              onClick={() => fileInputRef.current?.click()}
              size="sm"
              type="button"
              variant="secondary"
            >
              Choose JSON file
            </Button>
            <input
              accept="application/json,.json"
              className="hidden"
              data-testid="browser-share-import-file"
              onChange={onFileSelected}
              ref={fileInputRef}
              type="file"
            />
          </div>
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium"
              htmlFor="browser-share-import-json"
            >
              Share JSON
            </label>
            <Textarea
              className="min-h-[10rem] font-mono text-2xs"
              data-testid="browser-share-import-json"
              id="browser-share-import-json"
              onChange={(event) => setRaw(event.target.value)}
              placeholder='{"kind":"hula-browser-share.v1",...}'
              value={raw}
            />
          </div>
          {allowPin ? (
            <fieldset className="space-y-2">
              <legend className="text-sm font-medium">Import as</legend>
              <label className="flex items-center gap-2 text-sm">
                <input
                  checked={target === "session"}
                  data-testid="browser-share-import-as-session"
                  name="browser-share-import-target"
                  onChange={() => setTarget("session")}
                  type="radio"
                />
                Browser session (playground)
              </label>
              <label className="flex items-center gap-2 text-sm">
                <input
                  checked={target === "pin"}
                  data-testid="browser-share-import-as-pin"
                  name="browser-share-import-target"
                  onChange={() => setTarget("pin")}
                  type="radio"
                />
                Personal pinned site
              </label>
            </fieldset>
          ) : null}
          {error ? (
            <p className="text-sm text-destructive" role="alert">
              {error}
            </p>
          ) : null}
        </div>
      </ChooserDialogContent>
    </Dialog>
  );
}

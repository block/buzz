import { Dialog as BaseDialog } from "@base-ui/react/dialog";
import { useState } from "react";

import { Button } from "@/shared/ui/Button";

/** Composer-local link entry. It returns a label and URL; the editor applies them. */
export function ComposerLinkDialog({
  open,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (link: { label: string; url: string }) => void;
}) {
  const [label, setLabel] = useState("");
  const [url, setUrl] = useState("");
  return (
    <BaseDialog.Root open={open} onOpenChange={onOpenChange}>
      <BaseDialog.Portal>
        <BaseDialog.Backdrop className="composer-link-backdrop" />
        <BaseDialog.Popup className="composer-link-dialog">
          <BaseDialog.Title className="text-heading text-primary">
            Add link
          </BaseDialog.Title>
          <form
            onSubmit={(event) => {
              event.preventDefault();
              if (!url.trim()) return;
              onSubmit({ label: label.trim() || url.trim(), url: url.trim() });
              setLabel("");
              setUrl("");
              onOpenChange(false);
            }}
          >
            <label className="text-body text-primary">
              Link text
              <input
                value={label}
                onChange={(event) => setLabel(event.target.value)}
              />
            </label>
            <label className="text-body text-primary">
              Address
              <input
                required
                type="url"
                value={url}
                onChange={(event) => setUrl(event.target.value)}
              />
            </label>
            <div className="composer-link-actions">
              <BaseDialog.Close
                render={<Button variant="ghost">Cancel</Button>}
              />
              <Button type="submit" variant="primary">
                Add link
              </Button>
            </div>
          </form>
        </BaseDialog.Popup>
      </BaseDialog.Portal>
    </BaseDialog.Root>
  );
}

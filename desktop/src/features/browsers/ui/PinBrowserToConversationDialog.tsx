import * as React from "react";
import { toast } from "sonner";

import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

import {
  parseBrowserRowPinLink,
  pinBrowserRowToConversation,
  resolveBrowserRowPinTarget,
} from "../lib/pinBrowserToConversation";

export type PinBrowserDialogSource = {
  sid: string;
  title: string;
  url: string;
  pin?: string;
  stack?: string;
  expires?: string;
};

export function PinBrowserToConversationDialog({
  onOpenChange,
  onPinned,
  open,
  source,
}: {
  onOpenChange: (open: boolean) => void;
  onPinned?: (input: { scopeKey: string; channelId: string }) => void;
  open: boolean;
  source: PinBrowserDialogSource | null;
}) {
  const [url, setUrl] = React.useState("");
  const [name, setName] = React.useState("");
  const [link, setLink] = React.useState("");
  const [linkError, setLinkError] = React.useState<string | null>(null);
  const [channel, setChannel] = React.useState("");
  const [thread, setThread] = React.useState("");
  const [advancedOpen, setAdvancedOpen] = React.useState(false);
  const [formError, setFormError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open || !source) return;
    setUrl(source.url);
    setName(source.title);
    setLink("");
    setLinkError(null);
    setChannel("");
    setThread("");
    setAdvancedOpen(false);
    setFormError(null);
  }, [open, source]);

  function applyLink(value: string) {
    setLink(value);
    if (!value.trim()) {
      setLinkError(null);
      return;
    }
    const parsed = parseBrowserRowPinLink(value);
    if (!parsed) {
      setLinkError("Paste a Buzz channel or thread link.");
      return;
    }
    setLinkError(null);
    setChannel(parsed.channelId);
    setThread(parsed.threadRoot ?? "");
  }

  const canConfirm = Boolean(source && channel.trim() && url.trim());

  function handleSubmit(event: React.FormEvent) {
    event.preventDefault();
    if (!source) return;
    const target = resolveBrowserRowPinTarget({
      channelId: channel,
      threadRoot: thread.trim() || null,
    });
    if (!target) {
      setFormError("Choose a channel or paste a channel/thread link.");
      return;
    }
    const pinned = pinBrowserRowToConversation({
      sid: source.sid,
      title: source.title,
      url,
      name,
      channelId: target.channelId,
      threadRoot: target.threadRoot,
      ...(source.pin ? { pin: source.pin } : {}),
      ...(source.stack ? { stack: source.stack } : {}),
      ...(source.expires != null ? { expires: source.expires } : {}),
    });
    if (!pinned) {
      setFormError("Enter an http(s) URL and a channel.");
      return;
    }
    toast.success(target.threadRoot ? "Pinned to thread" : "Pinned to channel");
    onPinned?.({ scopeKey: target.scopeKey, channelId: target.channelId });
    onOpenChange(false);
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <ChooserDialogContent
        data-testid="browser-row-pin-dialog"
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
              data-testid="browser-row-pin-submit"
              disabled={!canConfirm}
              form="browser-row-pin-form"
              type="submit"
            >
              Pin
            </Button>
          </div>
        }
        title="Pin to channel or thread"
      >
        <form
          className="space-y-4"
          id="browser-row-pin-form"
          onSubmit={handleSubmit}
        >
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium"
              htmlFor="browser-row-pin-url"
            >
              URL
            </label>
            <Input
              autoFocus
              data-testid="browser-row-pin-url"
              id="browser-row-pin-url"
              onChange={(event) => setUrl(event.target.value)}
              placeholder="https://example.com"
              value={url}
            />
          </div>
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium"
              htmlFor="browser-row-pin-name"
            >
              Name{" "}
              <span className="font-normal text-muted-foreground">
                (optional)
              </span>
            </label>
            <Input
              data-testid="browser-row-pin-name"
              id="browser-row-pin-name"
              onChange={(event) => setName(event.target.value)}
              placeholder="Uses the site host when empty"
              value={name}
            />
          </div>
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium"
              htmlFor="browser-row-pin-link"
            >
              Channel or thread link
            </label>
            <Input
              className="font-mono text-xs"
              data-testid="browser-row-pin-link"
              id="browser-row-pin-link"
              onChange={(event) => applyLink(event.target.value)}
              placeholder="hulabuzz://message?… or /channels/…"
              value={link}
            />
            {linkError ? (
              <p className="text-2xs text-destructive">{linkError}</p>
            ) : null}
            {channel.trim() ? (
              <p
                className="truncate text-2xs text-muted-foreground"
                data-testid="browser-row-pin-link-preview"
              >
                Bound to {channel.trim()}
                {thread.trim()
                  ? ` · thread ${thread.trim().slice(0, 12)}…`
                  : ""}
              </p>
            ) : null}
          </div>
          <div>
            <button
              className="text-xs text-muted-foreground underline-offset-2 hover:underline"
              data-testid="browser-row-pin-advanced-toggle"
              onClick={() => setAdvancedOpen((value) => !value)}
              type="button"
            >
              {advancedOpen ? "Hide advanced" : "Advanced"}
            </button>
            {advancedOpen ? (
              <div
                className="mt-2 flex flex-col gap-2"
                data-testid="browser-row-pin-advanced"
              >
                <div className="space-y-1.5">
                  <label
                    className="text-sm font-medium"
                    htmlFor="browser-row-pin-channel"
                  >
                    Channel id
                  </label>
                  <Input
                    className="font-mono text-xs"
                    data-testid="browser-row-pin-channel"
                    id="browser-row-pin-channel"
                    onChange={(event) => setChannel(event.target.value)}
                    value={channel}
                  />
                </div>
                <div className="space-y-1.5">
                  <label
                    className="text-sm font-medium"
                    htmlFor="browser-row-pin-thread"
                  >
                    Thread root (optional)
                  </label>
                  <Input
                    className="font-mono text-xs"
                    data-testid="browser-row-pin-thread"
                    id="browser-row-pin-thread"
                    onChange={(event) => setThread(event.target.value)}
                    value={thread}
                  />
                </div>
              </div>
            ) : null}
          </div>
          {formError ? (
            <p className="text-sm text-destructive" role="alert">
              {formError}
            </p>
          ) : null}
        </form>
      </ChooserDialogContent>
    </Dialog>
  );
}

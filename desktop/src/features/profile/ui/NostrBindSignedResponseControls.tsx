import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

const COPY_BUTTON_LABEL_CLASS =
  "col-start-1 row-start-1 transition-[opacity,transform] duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:translate-y-0 motion-reduce:duration-0";

export function NostrBindSignedResponseControls({
  copyFailed,
  copyLabel,
  onCopy,
  signedResponse,
}: {
  copyFailed: boolean;
  copyLabel: string;
  onCopy: () => void;
  signedResponse: string;
}) {
  return (
    <div className="space-y-4" data-testid="nostr-bind-manual-fallback-content">
      <pre
        className="max-h-56 w-full overflow-auto rounded-2xl border border-border/70 bg-muted/60 p-4 text-left shadow-xs"
        data-testid="nostr-bind-signed-response"
      >
        <code className="whitespace-pre-wrap break-all font-mono text-xs leading-5 text-foreground">
          {signedResponse}
        </code>
      </pre>

      {copyFailed ? (
        <p className="w-full rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-left text-sm text-destructive">
          Buzz couldn't access the clipboard. Try again.
        </p>
      ) : null}

      <Button
        aria-label={copyLabel}
        className="h-10 w-full"
        data-testid="nostr-bind-copy-response"
        onClick={onCopy}
        type="button"
      >
        <span aria-live="polite" className="sr-only">
          {copyLabel}
        </span>
        <span
          aria-hidden="true"
          className="inline-grid h-5 place-items-center overflow-hidden"
        >
          <span
            className={cn(
              COPY_BUTTON_LABEL_CLASS,
              copyLabel === "Copy response"
                ? "translate-y-0 opacity-100"
                : "-translate-y-0.5 opacity-0",
            )}
          >
            Copy response
          </span>
          <span
            className={cn(
              COPY_BUTTON_LABEL_CLASS,
              copyLabel === "Copied"
                ? "translate-y-0 opacity-100"
                : "translate-y-0.5 opacity-0",
            )}
          >
            Copied
          </span>
        </span>
      </Button>
    </div>
  );
}

import { Check, Copy } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";

/** Icon button that copies a full commit hash with a brief check feedback. */
export function CopyCommitHashButton({
  className,
  hash,
}: {
  className?: string;
  hash: string;
}) {
  const [copied, setCopied] = React.useState(false);
  const handleCopy = React.useCallback(() => {
    void navigator.clipboard.writeText(hash).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2_000);
    });
  }, [hash]);

  return (
    <button
      aria-label="Copy commit hash"
      className={cn(
        "flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
        className,
      )}
      onClick={handleCopy}
      type="button"
    >
      {copied ? (
        <Check className="h-3.5 w-3.5 text-green-500" />
      ) : (
        <Copy className="h-3.5 w-3.5" />
      )}
    </button>
  );
}

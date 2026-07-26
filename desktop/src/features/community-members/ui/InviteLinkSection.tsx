import { Check, ChevronDown, Link2 } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { mintInvite } from "@/shared/api/invites";
import { writeTextToClipboard } from "@/shared/lib/clipboard";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Separator } from "@/shared/ui/separator";
import { Spinner } from "@/shared/ui/spinner";

const TTL_OPTIONS: { label: string; value: number }[] = [
  { label: "1 day", value: 24 * 60 * 60 },
  { label: "3 days", value: 3 * 24 * 60 * 60 },
  { label: "7 days", value: 7 * 24 * 60 * 60 },
  { label: "30 days", value: 30 * 24 * 60 * 60 },
];

export const DEFAULT_INVITE_TTL_SECS = TTL_OPTIONS[1].value;

type CopyStatus = "idle" | "copying" | "copied";

/**
 * Share-with-link footer for the community invite dialog.
 *
 * Each copy action mints a fresh stateless invite code and places its
 * shareable landing-page URL on the clipboard.
 */
export function InviteLinkSection({
  onTtlSecsChange,
  ttlSecs,
}: {
  onTtlSecsChange: (ttlSecs: number) => void;
  ttlSecs: number;
}) {
  const [copyStatus, setCopyStatus] = React.useState<CopyStatus>("idle");
  const ttlLabel =
    TTL_OPTIONS.find((option) => option.value === ttlSecs)?.label ?? "3 days";
  const copyLabel =
    copyStatus === "copying"
      ? "Copying…"
      : copyStatus === "copied"
        ? "Copied"
        : "Copy link";

  React.useEffect(() => {
    if (copyStatus !== "copied") return;
    const resetTimer = window.setTimeout(() => setCopyStatus("idle"), 2000);
    return () => window.clearTimeout(resetTimer);
  }, [copyStatus]);

  async function handleCopy() {
    if (copyStatus === "copying") return;
    setCopyStatus("copying");
    try {
      const invite = await mintInvite(ttlSecs);
      await writeTextToClipboard(invite.url);
      setCopyStatus("copied");
      toast.success("Invite link copied");
    } catch {
      setCopyStatus("idle");
      toast.error("Couldn’t copy the invite link. Try again.");
    }
  }

  return (
    <section className="pt-2" data-testid="community-invite-link-section">
      <div className="flex items-center gap-3">
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
          <Link2 aria-hidden="true" className="h-4 w-4" />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="text-sm font-medium">Share with a link</h3>
          <p className="text-xs text-secondary-foreground/75">
            Anyone with the link can join this community.
          </p>
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label="Choose invite expiry"
              className="h-8 shrink-0 gap-1.5 px-2 text-muted-foreground"
              data-testid="invite-link-ttl-trigger"
              disabled={copyStatus === "copying"}
              size="sm"
              type="button"
              variant="ghost"
            >
              {ttlLabel}
              <ChevronDown aria-hidden="true" className="h-3.5 w-3.5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-40">
            <DropdownMenuLabel>Expires after</DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuRadioGroup
              onValueChange={(value) => onTtlSecsChange(Number(value))}
              value={String(ttlSecs)}
            >
              {TTL_OPTIONS.map((option) => (
                <DropdownMenuRadioItem
                  data-testid={`invite-link-ttl-${option.value}`}
                  key={option.value}
                  value={String(option.value)}
                >
                  {option.label}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      <Separator className="my-4 bg-input/40" />
      <div className="flex justify-end">
        <Button
          className="shrink-0 border-border shadow-none"
          data-copy-status={copyStatus}
          data-testid="copy-invite-link"
          disabled={copyStatus === "copying"}
          onClick={() => void handleCopy()}
          size="sm"
          type="button"
          variant="outline"
        >
          {copyStatus === "copying" ? (
            <Spinner aria-hidden="true" className="h-4 w-4 border-2" />
          ) : copyStatus === "copied" ? (
            <Check aria-hidden="true" className="h-4 w-4" />
          ) : (
            <Link2 aria-hidden="true" className="h-4 w-4" />
          )}
          {copyLabel}
        </Button>
      </div>
    </section>
  );
}

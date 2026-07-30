import { Check, ChevronDown, Link2, Trash2 } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  listActiveGuestInvites,
  mintInvite,
  revokeInvite,
  type ActiveGuestInvite,
} from "@/shared/api/invites";
import { writeTextToClipboard } from "@/shared/lib/clipboard";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
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

const MAX_USE_OPTIONS: { label: string; value: number | null }[] = [
  { label: "No limit", value: null },
  { label: "1 use", value: 1 },
  { label: "3 uses", value: 3 },
  { label: "5 uses", value: 5 },
  { label: "10 uses", value: 10 },
  { label: "25 uses", value: 25 },
];

export const DEFAULT_INVITE_TTL_SECS = TTL_OPTIONS[1].value;

type CopyStatus = "idle" | "copying" | "copied";

function copyButtonLabel(status: CopyStatus): string {
  if (status === "copying") return "Copying…";
  if (status === "copied") return "Copied";
  return "Copy link";
}

function formatInviteExpiry(expiresAt: number): string {
  return new Date(expiresAt * 1_000).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

/**
 * Share-with-link footer for the community invite dialog.
 *
 * Each copy action mints a fresh database-backed invite code and places its
 * shareable landing-page URL on the clipboard. Community invites may be
 * unlimited or capped to a selected number of joins. Channel guest links
 * always admit exactly one identity.
 */
export function InviteLinkSection({
  channelId,
  onTtlSecsChange,
  ttlSecs,
}: {
  channelId?: string;
  onTtlSecsChange: (ttlSecs: number) => void;
  ttlSecs: number;
}) {
  const [copyStatus, setCopyStatus] = React.useState<CopyStatus>("idle");
  const [maxUses, setMaxUses] = React.useState<number | null>(null);
  const [activeGuestInvites, setActiveGuestInvites] = React.useState<
    ActiveGuestInvite[]
  >([]);
  const [guestInvitesLoading, setGuestInvitesLoading] = React.useState(false);
  const [guestInvitesLoadFailed, setGuestInvitesLoadFailed] =
    React.useState(false);
  const [revokingInviteId, setRevokingInviteId] = React.useState<string | null>(
    null,
  );
  const guestInviteLoadRequestRef = React.useRef(0);
  const ttlLabel =
    TTL_OPTIONS.find((option) => option.value === ttlSecs)?.label ?? "3 days";
  const maxUsesLabel =
    MAX_USE_OPTIONS.find((option) => option.value === maxUses)?.label ??
    "No limit";
  const copyLabel = copyButtonLabel(copyStatus);

  React.useEffect(() => {
    if (copyStatus !== "copied") return;
    const resetTimer = window.setTimeout(() => setCopyStatus("idle"), 2000);
    return () => window.clearTimeout(resetTimer);
  }, [copyStatus]);

  const loadGuestInvites = React.useCallback(async () => {
    if (!channelId) return;
    const requestId = ++guestInviteLoadRequestRef.current;
    setGuestInvitesLoading(true);
    setGuestInvitesLoadFailed(false);
    try {
      const invites = await listActiveGuestInvites(channelId);
      if (guestInviteLoadRequestRef.current === requestId) {
        setActiveGuestInvites(invites);
      }
    } catch {
      if (guestInviteLoadRequestRef.current === requestId) {
        setGuestInvitesLoadFailed(true);
      }
    } finally {
      if (guestInviteLoadRequestRef.current === requestId) {
        setGuestInvitesLoading(false);
      }
    }
  }, [channelId]);

  React.useEffect(() => {
    void loadGuestInvites();
  }, [loadGuestInvites]);

  async function handleCopy() {
    if (copyStatus === "copying") return;
    setCopyStatus("copying");
    let mintedGuestInviteId: string | null = null;
    try {
      const invite = await mintInvite({
        ttlSecs,
        maxUses: channelId ? 1 : maxUses,
        channelId,
      });
      mintedGuestInviteId = channelId ? invite.inviteId : null;
      await writeTextToClipboard(invite.url);
      if (channelId) {
        guestInviteLoadRequestRef.current += 1;
        setGuestInvitesLoading(false);
        setGuestInvitesLoadFailed(false);
        setActiveGuestInvites((current) => [
          {
            inviteId: invite.inviteId,
            expiresAt: invite.expiresAt,
            createdAt: Math.floor(Date.now() / 1_000),
          },
          ...current.filter((item) => item.inviteId !== invite.inviteId),
        ]);
      }
      setCopyStatus("copied");
      toast.success(channelId ? "Guest link copied" : "Invite link copied");
    } catch {
      if (mintedGuestInviteId) {
        try {
          await revokeInvite(mintedGuestInviteId);
        } catch {
          await loadGuestInvites();
        }
      }
      setCopyStatus("idle");
      toast.error("Couldn’t copy the invite link. Try again.");
    }
  }

  async function handleRevoke(inviteId: string) {
    if (revokingInviteId) return;
    setRevokingInviteId(inviteId);
    try {
      await revokeInvite(inviteId);
      guestInviteLoadRequestRef.current += 1;
      setGuestInvitesLoading(false);
      setActiveGuestInvites((current) =>
        current.filter((invite) => invite.inviteId !== inviteId),
      );
      toast.success("Guest link revoked");
    } catch {
      await loadGuestInvites();
      toast.error("Couldn’t revoke the guest link. Try again.");
    } finally {
      setRevokingInviteId(null);
    }
  }

  let activeGuestInvitesContent: React.ReactNode = null;
  if (guestInvitesLoadFailed) {
    activeGuestInvitesContent = (
      <div
        className="flex items-center justify-between gap-3 text-xs"
        role="alert"
      >
        <span>Active links could not be loaded.</span>
        <Button
          className="min-h-10"
          onClick={() => void loadGuestInvites()}
          size="sm"
          type="button"
          variant="ghost"
        >
          Retry
        </Button>
      </div>
    );
  } else if (!guestInvitesLoading && activeGuestInvites.length === 0) {
    activeGuestInvitesContent = (
      <p className="text-xs text-muted-foreground">No active guest links.</p>
    );
  } else {
    activeGuestInvitesContent = activeGuestInvites.map((invite) => {
      const expiryLabel = formatInviteExpiry(invite.expiresAt);
      return (
        <div
          className="flex items-center justify-between gap-3 rounded-lg border border-border/70 px-3 py-2"
          data-testid={`active-guest-invite-${invite.inviteId}`}
          key={invite.inviteId}
        >
          <span className="text-xs text-muted-foreground">
            Expires {expiryLabel}
          </span>
          <Button
            aria-label={`Revoke guest link expiring ${expiryLabel}`}
            className="min-h-10"
            disabled={revokingInviteId !== null}
            onClick={() => void handleRevoke(invite.inviteId)}
            size="sm"
            type="button"
            variant="ghost"
          >
            {revokingInviteId === invite.inviteId ? (
              <Spinner aria-hidden="true" className="h-4 w-4 border-2" />
            ) : (
              <Trash2 aria-hidden="true" className="h-4 w-4" />
            )}
            Revoke
          </Button>
        </div>
      );
    });
  }

  let copyIcon = <Link2 aria-hidden="true" className="h-4 w-4" />;
  if (copyStatus === "copying") {
    copyIcon = <Spinner aria-hidden="true" className="h-4 w-4 border-2" />;
  } else if (copyStatus === "copied") {
    copyIcon = <Check aria-hidden="true" className="h-4 w-4" />;
  }

  return (
    <section
      className="pt-2"
      data-testid={
        channelId
          ? "channel-guest-invite-link-section"
          : "community-invite-link-section"
      }
    >
      <div className="space-y-5">
        <div className="flex items-center justify-between gap-4">
          <span className="text-sm font-medium">Expires after</span>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                aria-label="Choose invite expiry"
                className="h-8 shrink-0 gap-1.5 px-2 text-sm text-muted-foreground"
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
        {channelId ? (
          <div className="flex items-center justify-between gap-4">
            <span className="text-sm font-medium">Number of uses</span>
            <span className="px-2 text-sm text-muted-foreground">1 use</span>
          </div>
        ) : (
          <div className="flex items-center justify-between gap-4">
            <span className="text-sm font-medium">Limit number of uses</span>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  aria-label="Choose maximum invite uses"
                  className="h-8 shrink-0 gap-1.5 px-2 text-sm text-muted-foreground"
                  data-testid="invite-link-max-uses-trigger"
                  disabled={copyStatus === "copying"}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  {maxUsesLabel}
                  <ChevronDown aria-hidden="true" className="h-3.5 w-3.5" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-40">
                <DropdownMenuRadioGroup
                  onValueChange={(value) =>
                    setMaxUses(value === "no-limit" ? null : Number(value))
                  }
                  value={String(maxUses ?? "no-limit")}
                >
                  {MAX_USE_OPTIONS.map((option) => (
                    <DropdownMenuRadioItem
                      data-testid={`invite-link-max-uses-${option.value ?? "no-limit"}`}
                      key={option.value ?? "no-limit"}
                      value={String(option.value ?? "no-limit")}
                    >
                      {option.label}
                    </DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )}
      </div>
      {channelId ? (
        <>
          <Separator className="my-4 bg-input/40" />
          <div className="space-y-2" data-testid="active-guest-invites">
            <div className="flex items-center justify-between gap-3">
              <span className="text-sm font-medium">Active guest links</span>
              {guestInvitesLoading ? (
                <Spinner
                  aria-label="Loading active guest links"
                  className="h-4 w-4 border-2"
                />
              ) : null}
            </div>
            <div className="max-h-60 space-y-2 overflow-y-auto pr-1">
              {activeGuestInvitesContent}
            </div>
          </div>
        </>
      ) : null}
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
          {copyIcon}
          {copyLabel}
        </Button>
      </div>
    </section>
  );
}

import { Check, ChevronDown, Link2, Trash2 } from "lucide-react";
import { motion, useReducedMotion } from "motion/react";
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
import { Input } from "@/shared/ui/input";
import { Separator } from "@/shared/ui/separator";
import { Spinner } from "@/shared/ui/spinner";

const TTL_OPTIONS = [
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
type GenerationStatus = "idle" | "generating" | "failed";
type InviteLinkSectionProps = {
  channelId?: string;
  onTtlSecsChange: (ttlSecs: number) => void;
  ttlSecs: number;
};

/** Community links are prepared on open. Guest links are minted only on copy. */
export function InviteLinkSection(props: InviteLinkSectionProps) {
  return props.channelId ? (
    <GuestInviteLinkSection {...props} channelId={props.channelId} />
  ) : (
    <CommunityInviteLinkSection {...props} />
  );
}

function CommunityInviteLinkSection({
  onTtlSecsChange,
  ttlSecs,
}: InviteLinkSectionProps) {
  const [copyStatus, setCopyStatus] = React.useState<CopyStatus>("idle");
  const [generationStatus, setGenerationStatus] =
    React.useState<GenerationStatus>("generating");
  const [inviteUrl, setInviteUrl] = React.useState("");
  const [maxUses, setMaxUses] = React.useState<number | null>(null);
  const generationRequestId = React.useRef(0);
  const inviteRequests = React.useRef(
    new Map<string, ReturnType<typeof mintInvite>>(),
  );
  const shouldReduceMotion = useReducedMotion();
  const isGenerating = generationStatus === "generating";
  const failed = generationStatus === "failed";
  const settingsKey = `${ttlSecs}:${maxUses ?? "no-limit"}`;
  const isWorking = isGenerating || copyStatus === "copying";
  const copyLabel = failed
    ? "Retry"
    : copyStatus === "copied"
      ? "Copied"
      : "Copy link";
  const copyButtonWidth = isWorking
    ? "6.25rem"
    : copyStatus === "copied"
      ? "5.25rem"
      : "4.5rem";

  React.useEffect(() => {
    if (copyStatus !== "copied") return;
    const timer = window.setTimeout(() => setCopyStatus("idle"), 2000);
    return () => window.clearTimeout(timer);
  }, [copyStatus]);

  const generateInviteLink = React.useCallback(async () => {
    const requestId = ++generationRequestId.current;
    setGenerationStatus("generating");
    setInviteUrl("");
    setCopyStatus("idle");
    const existing = inviteRequests.current.get(settingsKey);
    const request = existing ?? mintInvite({ ttlSecs, maxUses });
    if (!existing) inviteRequests.current.set(settingsKey, request);
    try {
      const invite = await request;
      if (inviteRequests.current.get(settingsKey) === request) {
        inviteRequests.current.delete(settingsKey);
      }
      if (generationRequestId.current === requestId) {
        setInviteUrl(invite.url);
        setGenerationStatus("idle");
      }
    } catch {
      if (inviteRequests.current.get(settingsKey) === request) {
        inviteRequests.current.delete(settingsKey);
      }
      if (generationRequestId.current === requestId) {
        setGenerationStatus("failed");
        toast.error("Couldn’t create an invite link.");
      }
    }
  }, [maxUses, settingsKey, ttlSecs]);

  React.useEffect(() => {
    void generateInviteLink();
    return () => {
      generationRequestId.current += 1;
    };
  }, [generateInviteLink]);

  async function handleCopy() {
    if (!inviteUrl || isGenerating || copyStatus === "copying") return;
    setCopyStatus("copying");
    try {
      await writeTextToClipboard(inviteUrl);
      setCopyStatus("copied");
      toast.success("Invite link copied");
    } catch {
      setCopyStatus("idle");
      toast.error("Couldn’t copy the invite link. Try again.");
    }
  }

  return (
    <section data-testid="community-invite-link-section">
      <div className="relative">
        <Input
          aria-label="Community invite link"
          className="h-11 pr-28 text-transparent caret-transparent selection:bg-transparent"
          data-testid="invite-link-url"
          disabled={isGenerating}
          placeholder={
            failed ? "Couldn’t create invite link" : "Creating invite link…"
          }
          readOnly
          value={inviteUrl}
        />
        {inviteUrl ? (
          <span
            aria-hidden="true"
            className="pointer-events-none absolute inset-y-0 left-3 right-28 flex items-center truncate text-sm text-muted-foreground"
            data-testid="invite-link-preview"
          >
            {inviteUrl}
          </span>
        ) : null}
        <motion.div
          animate={{ width: copyButtonWidth }}
          className="absolute right-1 top-1"
          initial={false}
          transition={
            shouldReduceMotion
              ? { duration: 0 }
              : { duration: 0.12, ease: [0.77, 0, 0.175, 1] as const }
          }
        >
          <Button
            className="h-9 w-full px-3"
            data-copy-status={copyStatus}
            data-testid="copy-invite-link"
            disabled={
              !failed &&
              (isGenerating || !inviteUrl || copyStatus === "copying")
            }
            onClick={() =>
              failed ? void generateInviteLink() : void handleCopy()
            }
            size="sm"
            type="button"
          >
            {isWorking ? (
              <Spinner aria-hidden="true" className="h-4 w-4 border-2" />
            ) : copyStatus === "copied" ? (
              <Check aria-hidden="true" className="h-4 w-4" />
            ) : null}
            {copyLabel}
          </Button>
        </motion.div>
      </div>
      <CommunityInviteSettings
        disabled={isGenerating || copyStatus === "copying"}
        maxUses={maxUses}
        onMaxUsesChange={setMaxUses}
        onTtlSecsChange={onTtlSecsChange}
        ttlSecs={ttlSecs}
      />
    </section>
  );
}

function GuestInviteLinkSection({
  channelId,
  onTtlSecsChange,
  ttlSecs,
}: InviteLinkSectionProps & { channelId: string }) {
  const [copyStatus, setCopyStatus] = React.useState<CopyStatus>("idle");
  const [invites, setInvites] = React.useState<ActiveGuestInvite[]>([]);
  const [loading, setLoading] = React.useState(false);
  const [loadFailed, setLoadFailed] = React.useState(false);
  const [revokingId, setRevokingId] = React.useState<string | null>(null);
  const loadRequestId = React.useRef(0);

  React.useEffect(() => {
    if (copyStatus !== "copied") return;
    const timer = window.setTimeout(() => setCopyStatus("idle"), 2000);
    return () => window.clearTimeout(timer);
  }, [copyStatus]);

  const loadInvites = React.useCallback(async () => {
    const requestId = ++loadRequestId.current;
    setLoading(true);
    setLoadFailed(false);
    try {
      const result = await listActiveGuestInvites(channelId);
      if (loadRequestId.current === requestId) setInvites(result);
    } catch {
      if (loadRequestId.current === requestId) setLoadFailed(true);
    } finally {
      if (loadRequestId.current === requestId) setLoading(false);
    }
  }, [channelId]);

  React.useEffect(() => {
    void loadInvites();
  }, [loadInvites]);

  async function handleCopy() {
    if (copyStatus === "copying") return;
    setCopyStatus("copying");
    let inviteId: string | null = null;
    try {
      const invite = await mintInvite({ channelId, maxUses: 1, ttlSecs });
      inviteId = invite.inviteId;
      await writeTextToClipboard(invite.url);
      loadRequestId.current += 1;
      setLoading(false);
      setLoadFailed(false);
      setInvites((current) => [
        {
          createdAt: Math.floor(Date.now() / 1_000),
          expiresAt: invite.expiresAt,
          inviteId: invite.inviteId,
        },
        ...current.filter((item) => item.inviteId !== invite.inviteId),
      ]);
      setCopyStatus("copied");
      toast.success("Guest link copied");
    } catch {
      if (inviteId) {
        try {
          await revokeInvite(inviteId);
        } catch {
          await loadInvites();
        }
      }
      setCopyStatus("idle");
      toast.error("Couldn’t copy the invite link. Try again.");
    }
  }

  async function handleRevoke(inviteId: string) {
    if (revokingId) return;
    setRevokingId(inviteId);
    try {
      await revokeInvite(inviteId);
      loadRequestId.current += 1;
      setLoading(false);
      setInvites((current) =>
        current.filter((invite) => invite.inviteId !== inviteId),
      );
      toast.success("Guest link revoked");
    } catch {
      await loadInvites();
      toast.error("Couldn’t revoke the guest link. Try again.");
    } finally {
      setRevokingId(null);
    }
  }

  return (
    <section className="pt-2" data-testid="channel-guest-invite-link-section">
      <div className="space-y-3">
        <div className="flex items-center justify-between gap-4">
          <span className="text-sm font-medium">Expires after</span>
          <TtlMenu
            disabled={copyStatus === "copying"}
            onChange={onTtlSecsChange}
            value={ttlSecs}
          />
        </div>
        <div className="flex items-center justify-between gap-4">
          <span className="text-sm font-medium">Number of uses</span>
          <span className="px-2 text-sm text-muted-foreground">1 use</span>
        </div>
      </div>
      <Separator className="my-4 bg-input/40" />
      <div className="space-y-2" data-testid="active-guest-invites">
        <div className="flex items-center justify-between gap-3">
          <span className="text-sm font-medium">Active guest links</span>
          {loading ? (
            <Spinner
              aria-label="Loading active guest links"
              className="h-4 w-4 border-2"
            />
          ) : null}
        </div>
        <div className="max-h-60 space-y-2 overflow-y-auto pr-1">
          {loadFailed ? (
            <div
              className="flex items-center justify-between gap-3 text-xs"
              role="alert"
            >
              <span>Active links could not be loaded.</span>
              <Button
                className="min-h-10"
                onClick={() => void loadInvites()}
                size="sm"
                type="button"
                variant="ghost"
              >
                Retry
              </Button>
            </div>
          ) : !loading && invites.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              No active guest links.
            </p>
          ) : (
            invites.map((invite) => (
              <GuestInviteRow
                invite={invite}
                key={invite.inviteId}
                onRevoke={handleRevoke}
                revoking={revokingId !== null}
                revokingThis={revokingId === invite.inviteId}
              />
            ))
          )}
        </div>
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
          {copyStatus === "copying"
            ? "Copying…"
            : copyStatus === "copied"
              ? "Copied"
              : "Copy link"}
        </Button>
      </div>
    </section>
  );
}

function GuestInviteRow({
  invite,
  onRevoke,
  revoking,
  revokingThis,
}: {
  invite: ActiveGuestInvite;
  onRevoke: (inviteId: string) => Promise<void>;
  revoking: boolean;
  revokingThis: boolean;
}) {
  const expiry = new Date(invite.expiresAt * 1_000).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
  return (
    <div
      className="flex items-center justify-between gap-3 rounded-lg border border-border/70 px-3 py-2"
      data-testid={`active-guest-invite-${invite.inviteId}`}
    >
      <span className="text-xs text-muted-foreground">Expires {expiry}</span>
      <Button
        aria-label={`Revoke guest link expiring ${expiry}`}
        className="min-h-10"
        disabled={revoking}
        onClick={() => void onRevoke(invite.inviteId)}
        size="sm"
        type="button"
        variant="ghost"
      >
        {revokingThis ? (
          <Spinner aria-hidden="true" className="h-4 w-4 border-2" />
        ) : (
          <Trash2 aria-hidden="true" className="h-4 w-4" />
        )}
        Revoke
      </Button>
    </div>
  );
}

function CommunityInviteSettings({
  disabled,
  maxUses,
  onMaxUsesChange,
  onTtlSecsChange,
  ttlSecs,
}: {
  disabled: boolean;
  maxUses: number | null;
  onMaxUsesChange: (value: number | null) => void;
  onTtlSecsChange: (value: number) => void;
  ttlSecs: number;
}) {
  const maxUsesLabel =
    MAX_USE_OPTIONS.find((option) => option.value === maxUses)?.label ??
    "No limit";
  return (
    <div className="mt-3 space-y-3">
      <div className="flex items-center justify-between gap-4">
        <span className="text-sm font-medium">Expires after</span>
        <TtlMenu
          disabled={disabled}
          onChange={onTtlSecsChange}
          value={ttlSecs}
        />
      </div>
      <div className="flex items-center justify-between gap-4">
        <span className="text-sm font-medium">Limit number of uses</span>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label="Choose maximum invite uses"
              className="h-8 shrink-0 gap-1.5 px-2 text-sm text-muted-foreground"
              data-testid="invite-link-max-uses-trigger"
              disabled={disabled}
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
                onMaxUsesChange(value === "no-limit" ? null : Number(value))
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
    </div>
  );
}

function TtlMenu({
  disabled,
  onChange,
  value,
}: {
  disabled: boolean;
  onChange: (value: number) => void;
  value: number;
}) {
  const label =
    TTL_OPTIONS.find((option) => option.value === value)?.label ?? "3 days";
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          aria-label="Choose invite expiry"
          className="h-8 shrink-0 gap-1.5 px-2 text-sm text-muted-foreground"
          data-testid="invite-link-ttl-trigger"
          disabled={disabled}
          size="sm"
          type="button"
          variant="ghost"
        >
          {label}
          <ChevronDown aria-hidden="true" className="h-3.5 w-3.5" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-40">
        <DropdownMenuRadioGroup
          onValueChange={(nextValue) => onChange(Number(nextValue))}
          value={String(value)}
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
  );
}

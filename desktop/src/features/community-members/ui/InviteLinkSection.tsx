import { Check, ChevronDown } from "lucide-react";
import { motion, useReducedMotion } from "motion/react";
import * as React from "react";
import { toast } from "sonner";

import { i18n, useTranslation } from "@/i18n";
import { mintInvite } from "@/shared/api/invites";
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
import { Spinner } from "@/shared/ui/spinner";

const TTL_OPTIONS: { id: string; value: number }[] = [
  { id: "1-day", value: 24 * 60 * 60 },
  { id: "3-days", value: 3 * 24 * 60 * 60 },
  { id: "7-days", value: 7 * 24 * 60 * 60 },
  { id: "30-days", value: 30 * 24 * 60 * 60 },
];

const MAX_USE_OPTIONS: { id: string; value: number | null }[] = [
  { id: "no-limit", value: null },
  { id: "1-use", value: 1 },
  { id: "3-uses", value: 3 },
  { id: "5-uses", value: 5 },
  { id: "10-uses", value: 10 },
  { id: "25-uses", value: 25 },
];

/** Option ids only — the `value`s are wire data and each label resolves at the
 *  render site through a literal `t()` key. */
function ttlLabel(id: string): string {
  switch (id) {
    case "3-days":
      return i18n.t("members.invite.ttl-3-days");
    case "7-days":
      return i18n.t("members.invite.ttl-7-days");
    case "30-days":
      return i18n.t("members.invite.ttl-30-days");
    default:
      return i18n.t("members.invite.ttl-1-day");
  }
}

function maxUsesLabel(id: string): string {
  switch (id) {
    case "1-use":
      return i18n.t("members.invite.uses-1");
    case "3-uses":
      return i18n.t("members.invite.uses-3");
    case "5-uses":
      return i18n.t("members.invite.uses-5");
    case "10-uses":
      return i18n.t("members.invite.uses-10");
    case "25-uses":
      return i18n.t("members.invite.uses-25");
    default:
      return i18n.t("members.invite.no-limit");
  }
}

export const DEFAULT_INVITE_TTL_SECS = TTL_OPTIONS[1].value;

type CopyStatus = "idle" | "copying" | "copied";
type GenerationStatus = "idle" | "generating" | "failed";

/**
 * Share-with-link footer for the community invite dialog.
 *
 * A database-backed invite link is minted when this section opens and whenever
 * its settings change. Invites may be unlimited or capped to a caller-selected
 * number of successful joins.
 */
export function InviteLinkSection({
  onTtlSecsChange,
  ttlSecs,
}: {
  onTtlSecsChange: (ttlSecs: number) => void;
  ttlSecs: number;
}) {
  const { t } = useTranslation();
  const [copyStatus, setCopyStatus] = React.useState<CopyStatus>("idle");
  const [generationStatus, setGenerationStatus] =
    React.useState<GenerationStatus>("generating");
  const [inviteUrl, setInviteUrl] = React.useState("");
  const [maxUses, setMaxUses] = React.useState<number | null>(null);
  const generationRequestId = React.useRef(0);
  // React StrictMode replays effects in development. Keep one in-flight mint
  // per setting set so the replay observes the original request instead of
  // creating a second durable invite.
  const inviteRequests = React.useRef(
    new Map<string, ReturnType<typeof mintInvite>>(),
  );
  const shouldReduceMotion = useReducedMotion();
  const resolvedTtlLabel = ttlLabel(
    TTL_OPTIONS.find((option) => option.value === ttlSecs)?.id ?? "3-days",
  );
  const resolvedMaxUsesLabel = maxUsesLabel(
    MAX_USE_OPTIONS.find((option) => option.value === maxUses)?.id ??
      "no-limit",
  );
  const isGenerating = generationStatus === "generating";
  const hasGenerationFailed = generationStatus === "failed";
  const inviteSettingsKey = `${ttlSecs}:${maxUses ?? "no-limit"}`;
  const isWorking = isGenerating || copyStatus === "copying";
  const copyLabel = hasGenerationFailed
    ? t("members.invite.retry")
    : copyStatus === "copied"
      ? t("members.invite.copied")
      : t("members.invite.copy-link");
  const copyButtonWidth = isWorking
    ? "6.25rem"
    : copyStatus === "copied"
      ? "5.25rem"
      : "4.5rem";
  const copyButtonTransition = shouldReduceMotion
    ? { duration: 0 }
    : { duration: 0.12, ease: [0.77, 0, 0.175, 1] as const };

  React.useEffect(() => {
    if (copyStatus !== "copied") return;
    const resetTimer = window.setTimeout(() => setCopyStatus("idle"), 2000);
    return () => window.clearTimeout(resetTimer);
  }, [copyStatus]);

  const generateInviteLink = React.useCallback(async () => {
    const requestId = generationRequestId.current + 1;
    generationRequestId.current = requestId;
    setGenerationStatus("generating");
    setInviteUrl("");
    setCopyStatus("idle");
    const existingRequest = inviteRequests.current.get(inviteSettingsKey);
    const inviteRequest = existingRequest ?? mintInvite({ ttlSecs, maxUses });
    if (!existingRequest) {
      inviteRequests.current.set(inviteSettingsKey, inviteRequest);
    }

    try {
      const invite = await inviteRequest;
      if (inviteRequests.current.get(inviteSettingsKey) === inviteRequest) {
        inviteRequests.current.delete(inviteSettingsKey);
      }
      if (generationRequestId.current === requestId) {
        setInviteUrl(invite.url);
        setGenerationStatus("idle");
      }
    } catch {
      if (inviteRequests.current.get(inviteSettingsKey) === inviteRequest) {
        inviteRequests.current.delete(inviteSettingsKey);
      }
      if (generationRequestId.current === requestId) {
        setGenerationStatus("failed");
        toast.error(t("members.invite.create-failed"));
      }
    }
  }, [inviteSettingsKey, maxUses, t, ttlSecs]);

  React.useEffect(() => {
    void generateInviteLink();
    return () => {
      generationRequestId.current += 1;
    };
  }, [generateInviteLink]);

  function retryInviteGeneration() {
    if (!hasGenerationFailed) return;
    void generateInviteLink();
  }

  async function handleCopy() {
    if (!inviteUrl || isGenerating || copyStatus === "copying") return;
    setCopyStatus("copying");
    try {
      await writeTextToClipboard(inviteUrl);
      setCopyStatus("copied");
      toast.success(t("members.invite.copied-toast"));
    } catch {
      setCopyStatus("idle");
      toast.error(t("members.invite.copy-failed"));
    }
  }

  return (
    <section data-testid="community-invite-link-section">
      <div className="relative">
        <Input
          aria-label={t("members.invite.link-label")}
          className="h-11 pr-28 text-transparent caret-transparent selection:bg-transparent"
          data-testid="invite-link-url"
          disabled={isGenerating}
          placeholder={
            hasGenerationFailed
              ? t("members.invite.create-failed-placeholder")
              : t("members.invite.creating")
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
          className="absolute right-1 top-1"
          animate={{ width: copyButtonWidth }}
          initial={false}
          transition={copyButtonTransition}
        >
          <Button
            className="h-9 w-full px-3"
            data-copy-status={copyStatus}
            data-testid="copy-invite-link"
            disabled={
              !hasGenerationFailed &&
              (isGenerating || !inviteUrl || copyStatus === "copying")
            }
            onClick={() =>
              hasGenerationFailed ? retryInviteGeneration() : void handleCopy()
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

      <div className="mt-3 space-y-3">
        <div className="flex items-center justify-between gap-4">
          <span className="text-sm font-medium">
            {t("members.invite.expires-after")}
          </span>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                aria-label={t("members.invite.choose-expiry")}
                className="h-8 shrink-0 gap-1.5 px-2 text-sm text-muted-foreground"
                data-testid="invite-link-ttl-trigger"
                disabled={isGenerating || copyStatus === "copying"}
                size="sm"
                type="button"
                variant="ghost"
              >
                {resolvedTtlLabel}
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
                    {ttlLabel(option.id)}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
        <div className="flex items-center justify-between gap-4">
          <span className="text-sm font-medium">
            {t("members.invite.limit-uses")}
          </span>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                aria-label={t("members.invite.choose-max-uses")}
                className="h-8 shrink-0 gap-1.5 px-2 text-sm text-muted-foreground"
                data-testid="invite-link-max-uses-trigger"
                disabled={isGenerating || copyStatus === "copying"}
                size="sm"
                type="button"
                variant="ghost"
              >
                {resolvedMaxUsesLabel}
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
                    {maxUsesLabel(option.id)}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
    </section>
  );
}

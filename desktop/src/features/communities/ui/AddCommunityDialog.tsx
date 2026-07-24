import * as React from "react";
import { ArrowLeft, ChevronRight, Link2, Plus } from "lucide-react";

import type { AddCommunityPrefillRequest } from "@/features/communities/addCommunityPrefill";
import { HostedCommunityCreateFlow } from "@/features/communities/ui/HostedCommunityCreateFlow";
import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import { InviteRedeemForm } from "@/features/onboarding/ui/InviteRedeemForm";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

type AddCommunityDialogProps = {
  prefill?: AddCommunityPrefillRequest | null;
  onSubmit?: (
    community: import("@/features/communities/types").Community,
  ) => void;
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

type AddCommunityMode = "choose" | "create" | "join";

const OPTION_CLASS =
  "flex w-full items-center gap-3 rounded-xl border border-border/70 bg-muted/30 px-4 py-4 text-left transition-colors duration-150 ease-out hover:bg-muted/60 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring";

export function AddCommunityDialog({
  prefill,
  open,
  onOpenChange,
}: AddCommunityDialogProps) {
  const communityOnboarding = useCommunityOnboarding();
  const [mode, setMode] = React.useState<AddCommunityMode>("choose");
  const [joinError, setJoinError] = React.useState<string | null>(null);
  const appliedPrefillId = React.useRef<string | null>(null);

  React.useEffect(() => {
    if (!prefill || appliedPrefillId.current === prefill.requestId) return;
    appliedPrefillId.current = prefill.requestId;
    setJoinError(null);
    setMode("join");
  }, [prefill]);

  const handleClose = React.useCallback(() => {
    onOpenChange(false);
    setMode("choose");
    setJoinError(null);
  }, [onOpenChange]);

  const startConnection = React.useCallback(
    ({
      relayUrl,
      inviteCode,
      policyReceipt,
    }: {
      relayUrl: string;
      inviteCode?: string;
      policyReceipt?: string;
    }) => {
      const started = communityOnboarding.start({
        source: "add-community",
        relayUrl,
        inviteCode,
        communityName: prefill?.name,
        policyReceipt,
      });
      if (!started) {
        setJoinError(
          "Finish connecting the community already in progress, then try again.",
        );
        return;
      }
      handleClose();
    },
    [communityOnboarding, handleClose, prefill?.name],
  );

  const title =
    mode === "create"
      ? "Create a new community"
      : mode === "join"
        ? "Join an existing community"
        : "Add community";

  const description =
    mode === "create"
      ? "Opens Builderlab in your browser."
      : mode === "join"
        ? "Use the community URL or invite link you received."
        : "Create a new community or join one you already have.";

  return (
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen) handleClose();
        else onOpenChange(true);
      }}
      open={open}
    >
      <DialogContent
        className="max-w-lg gap-0 overflow-hidden p-0"
        data-testid="add-community-dialog"
      >
        <DialogHeader className="px-6 pb-3 pt-5 pr-14">
          <div className="flex min-w-0 items-center gap-2">
            {mode !== "choose" ? (
              <button
                aria-label="Back to add community options"
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors duration-150 ease-out hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
                data-testid="add-community-back"
                onClick={() => {
                  setJoinError(null);
                  setMode("choose");
                }}
                type="button"
              >
                <ArrowLeft className="h-4 w-4" />
              </button>
            ) : null}
            <DialogTitle className="truncate">{title}</DialogTitle>
          </div>
          <DialogDescription
            className={mode === "create" ? "sr-only" : undefined}
          >
            {description}
          </DialogDescription>
        </DialogHeader>

        <div className="px-6 pb-6 pt-3">
          {mode === "choose" ? (
            <div className="space-y-3">
              <button
                className={OPTION_CLASS}
                data-testid="add-community-create"
                onClick={() => setMode("create")}
                type="button"
              >
                <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
                  <Plus className="h-4 w-4" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block text-sm font-medium text-foreground">
                    Create a new community
                  </span>
                  <span className="mt-0.5 block text-xs leading-5 text-muted-foreground">
                    Claim a Buzz address for your team.
                  </span>
                </span>
                <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground/60" />
              </button>

              <button
                className={OPTION_CLASS}
                data-testid="add-community-join"
                onClick={() => setMode("join")}
                type="button"
              >
                <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
                  <Link2 className="h-4 w-4" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block text-sm font-medium text-foreground">
                    Join an existing community
                  </span>
                  <span className="mt-0.5 block text-xs leading-5 text-muted-foreground">
                    Use a community URL or invite link.
                  </span>
                </span>
                <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground/60" />
              </button>
            </div>
          ) : mode === "join" ? (
            <InviteRedeemForm
              error={joinError}
              initialValue={prefill?.relayUrl}
              isRedeeming={false}
              key={prefill?.requestId ?? "manual-add-community"}
              onCancel={() => {
                setJoinError(null);
                setMode("choose");
              }}
              onConnect={(relayUrl) => startConnection({ relayUrl })}
              onRedeem={(relayUrl, inviteCode, policyReceipt) =>
                startConnection({ relayUrl, inviteCode, policyReceipt })
              }
              variant="add-community"
            />
          ) : (
            <HostedCommunityCreateFlow onComplete={handleClose} />
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

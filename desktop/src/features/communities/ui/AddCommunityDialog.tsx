import * as React from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import {
  deriveCommunityName,
  expandTilde,
  normalizeRelayUrl,
} from "@/features/communities/communityStorage";
import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import { inviteErrorMessage } from "@/shared/api/inviteHelpers";
import {
  acceptJoinPolicy,
  getJoinPolicy,
  isJoinPolicyDiscoveryCandidate,
  type JoinPolicy,
} from "@/shared/api/invites";
import { validateReposDir } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { JoinPolicyNotice } from "@/features/onboarding/ui/JoinPolicyNotice";

const POLICY_DISCOVERY_DELAY_MS = 250;
const POLICY_REVEAL_EASE = [0.23, 1, 0.32, 1] as const;

type AddCommunityDialogProps = {
  onSubmit?: (
    community: import("@/features/communities/types").Community,
  ) => void;
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

export function AddCommunityDialog({
  open,
  onOpenChange,
}: AddCommunityDialogProps) {
  const [name, setName] = React.useState("");
  const [relayUrl, setRelayUrl] = React.useState("");
  const [token, setToken] = React.useState("");
  const [inviteCode, setInviteCode] = React.useState("");
  const [inviteError, setInviteError] = React.useState<string | null>(null);
  const [joinPolicy, setJoinPolicy] = React.useState<JoinPolicy | null>(null);
  const [ageConfirmed, setAgeConfirmed] = React.useState(false);
  const [agreementConfirmed, setAgreementConfirmed] = React.useState(false);
  const [reposDir, setReposDir] = React.useState("");
  const communityOnboarding = useCommunityOnboarding();
  const [reposDirError, setReposDirError] = React.useState<string | null>(null);
  const shouldReduceMotion = useReducedMotion();

  React.useEffect(() => {
    if (!open || !relayUrl.trim()) return;

    const normalizedUrl = normalizeRelayUrl(relayUrl.trim());
    if (!isJoinPolicyDiscoveryCandidate(normalizedUrl)) return;

    let cancelled = false;
    const timeoutId = window.setTimeout(() => {
      void getJoinPolicy(normalizedUrl)
        .then((policy) => {
          if (cancelled || !policy) return;
          setJoinPolicy(policy);
          setAgeConfirmed(false);
          setAgreementConfirmed(false);
          setInviteError(null);
        })
        .catch(() => {
          // Background discovery is best-effort. A deliberate submit retries
          // the request and surfaces any relay error to the user.
        });
    }, POLICY_DISCOVERY_DELAY_MS);

    return () => {
      cancelled = true;
      window.clearTimeout(timeoutId);
    };
  }, [open, relayUrl]);

  const handleClose = React.useCallback(() => {
    onOpenChange(false);
    setName("");
    setRelayUrl("");
    setToken("");
    setInviteCode("");
    setInviteError(null);
    setJoinPolicy(null);
    setAgeConfirmed(false);
    setAgreementConfirmed(false);
    setReposDir("");
    setReposDirError(null);
  }, [onOpenChange]);

  const handleSubmit = React.useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      if (!relayUrl.trim()) {
        return;
      }

      // Expand `~` before save — the backend rejects tilde paths. Empty input
      // resolves to `undefined` so REPOS keeps its default location. Validate
      // the expanded value (the bytes the backend canonicalizes) before save
      // so a bad path is caught here instead of bricking a later boot.
      const expandedReposDir = await expandTilde(reposDir);
      try {
        await validateReposDir(expandedReposDir ?? "");
      } catch (error) {
        setReposDirError(String(error));
        return;
      }

      const normalizedRelayUrl = normalizeRelayUrl(relayUrl.trim());
      let policyReceipt: string | undefined;
      try {
        const policy = await getJoinPolicy(normalizedRelayUrl);
        if (policy && (!joinPolicy || joinPolicy.version !== policy.version)) {
          setJoinPolicy(policy);
          setAgeConfirmed(false);
          setAgreementConfirmed(false);
          setInviteError(null);
          return;
        }
        if (policy?.ageAttestationRequired && !ageConfirmed) {
          setInviteError("Confirm that you are at least 18 years old.");
          return;
        }
        if (
          policy &&
          (policy.termsMarkdown || policy.privacyMarkdown) &&
          !agreementConfirmed
        ) {
          setInviteError("Agree to the Terms of Service and Privacy Policy.");
          return;
        }

        // Receipts are bound to an invite code, so one is only minted when a
        // code is present. The claim itself runs on the onboarding
        // transaction (useClaimInvite), which forwards this receipt.
        if (policy && inviteCode.trim()) {
          policyReceipt = await acceptJoinPolicy(
            normalizedRelayUrl,
            inviteCode.trim(),
            policy.version,
            ageConfirmed,
          );
        }
      } catch (error) {
        setInviteError(`Community rejected: ${inviteErrorMessage(error)}`);
        return;
      }

      communityOnboarding.start({
        source: "add-community",
        relayUrl: normalizedRelayUrl,
        inviteCode: inviteCode.trim() || undefined,
        communityName: name.trim() || deriveCommunityName(normalizedRelayUrl),
        token: token.trim() || undefined,
        reposDir: expandedReposDir,
        policyReceipt,
      });
      handleClose();
    },
    [
      name,
      relayUrl,
      token,
      inviteCode,
      reposDir,
      joinPolicy,
      ageConfirmed,
      agreementConfirmed,
      communityOnboarding,
      handleClose,
    ],
  );

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Add Community</DialogTitle>
          <DialogDescription>
            Connect to another Buzz relay. Each community has its own channels,
            messages, and identity.
          </DialogDescription>
        </DialogHeader>
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => void handleSubmit(e)}
        >
          <div className="flex flex-col gap-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="ws-relay-url"
            >
              Relay URL
            </label>
            <Input
              autoFocus
              id="ws-relay-url"
              onChange={(e) => {
                setRelayUrl(e.target.value);
                setInviteError(null);
                setJoinPolicy(null);
                setAgeConfirmed(false);
                setAgreementConfirmed(false);
              }}
              placeholder="wss://relay.example.com"
              type="text"
              value={relayUrl}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="ws-name"
            >
              Name
              <span className="ml-1 text-xs font-normal text-muted-foreground">
                (optional)
              </span>
            </label>
            <Input
              id="ws-name"
              onChange={(e) => setName(e.target.value)}
              placeholder="My Community"
              type="text"
              value={name}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="ws-token"
            >
              API Token
              <span className="ml-1 text-xs font-normal text-muted-foreground">
                (optional)
              </span>
            </label>
            <Input
              id="ws-token"
              onChange={(e) => setToken(e.target.value)}
              placeholder="buzz_..."
              type="password"
              value={token}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="ws-invite-code"
            >
              Invite Code
              <span className="ml-1 text-xs font-normal text-muted-foreground">
                (optional)
              </span>
            </label>
            <Input
              id="ws-invite-code"
              onChange={(e) => {
                setInviteCode(e.target.value);
                setInviteError(null);
                setJoinPolicy(null);
                setAgeConfirmed(false);
                setAgreementConfirmed(false);
              }}
              placeholder="Paste an invite code for a members-only relay"
              type="text"
              value={inviteCode}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="ws-repos-dir"
            >
              Repos Directory
              <span className="ml-1 text-xs font-normal text-muted-foreground">
                (optional)
              </span>
            </label>
            <Input
              id="ws-repos-dir"
              onChange={(e) => {
                setReposDir(e.target.value);
                setReposDirError(null);
              }}
              placeholder="~/Development"
              type="text"
              value={reposDir}
            />
            {reposDirError ? (
              <p className="text-xs text-destructive">{reposDirError}</p>
            ) : null}
            <p className="text-xs text-muted-foreground">
              Point the agent's <code>REPOS</code> directory at an existing
              folder so agents work in your local checkouts. Leave blank to use
              the default location.
            </p>
          </div>
          <p className="text-xs text-muted-foreground">
            Communities share your active identity. To use a different key,
            import it on the profile step (or in settings).
          </p>
          {inviteError ? (
            <p className="text-xs text-destructive">{inviteError}</p>
          ) : null}
          <AnimatePresence initial={false}>
            {joinPolicy && relayUrl.trim() ? (
              <motion.div
                animate={{
                  height: "auto",
                  marginTop: 0,
                  opacity: 1,
                  transform: "translateY(0rem)",
                }}
                className="overflow-hidden"
                exit={
                  shouldReduceMotion
                    ? { height: 0, marginTop: "-1rem", opacity: 0 }
                    : {
                        height: 0,
                        marginTop: "-1rem",
                        opacity: 0,
                        transform: "translateY(-0.25rem)",
                      }
                }
                initial={
                  shouldReduceMotion
                    ? false
                    : {
                        height: 0,
                        marginTop: "-1rem",
                        opacity: 0,
                        transform: "translateY(-0.25rem)",
                      }
                }
                key={`${normalizeRelayUrl(relayUrl.trim())}:${joinPolicy.version}`}
                transition={
                  shouldReduceMotion
                    ? { duration: 0 }
                    : { duration: 0.22, ease: POLICY_REVEAL_EASE }
                }
              >
                <JoinPolicyNotice
                  ageConfirmed={ageConfirmed}
                  agreementConfirmed={agreementConfirmed}
                  onAgeConfirmedChange={(confirmed) => {
                    setAgeConfirmed(confirmed);
                    setInviteError(null);
                  }}
                  onAgreementConfirmedChange={(confirmed) => {
                    setAgreementConfirmed(confirmed);
                    setInviteError(null);
                  }}
                  policy={joinPolicy}
                  // Editing the relay URL resets joinPolicy, so a visible
                  // notice always belongs to the URL currently in the field.
                  relayWsUrl={normalizeRelayUrl(relayUrl.trim())}
                />
              </motion.div>
            ) : null}
          </AnimatePresence>
          <div className="flex justify-end gap-2 pt-2">
            <Button onClick={handleClose} type="button" variant="outline">
              Cancel
            </Button>
            <Button
              disabled={
                !relayUrl.trim() ||
                Boolean(joinPolicy?.ageAttestationRequired && !ageConfirmed) ||
                Boolean(
                  joinPolicy &&
                    (joinPolicy.termsMarkdown || joinPolicy.privacyMarkdown) &&
                    !agreementConfirmed,
                )
              }
              type="submit"
            >
              Add Community
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

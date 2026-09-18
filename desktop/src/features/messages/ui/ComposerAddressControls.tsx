import { AgentManagementMarker } from "@/features/agents/ui/OtherSetupAgentMarker";
import { ArrowUp, AtSign, Square, X } from "lucide-react";
import {
  AnimatePresence,
  motion,
  useAnimationControls,
  useReducedMotion,
} from "motion/react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { Popover, PopoverAnchor, PopoverContent } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export type ComposerAddressAgent = {
  avatarUrl: string | null;
  displayName: string;
  pubkey: string;
};

type AddressAnimationProps = {
  pulseVersion: number;
  shakeVersion: number;
};

function AddressedAgentAvatar({
  agent,
  pulseVersion,
  shakeVersion,
}: AddressAnimationProps & { agent: ComposerAddressAgent }) {
  const controls = useAnimationControls();
  const shouldReduceMotion = useReducedMotion();
  const previousPulseVersionRef = React.useRef(0);
  const previousShakeVersionRef = React.useRef(0);

  React.useEffect(() => {
    if (pulseVersion <= previousPulseVersionRef.current) return;
    previousPulseVersionRef.current = pulseVersion;
    if (shouldReduceMotion) return;
    void controls.start({
      scale: [1, 1.3, 0.96, 1.08, 1],
      y: [0, -4, 1, -1, 0],
      transition: { duration: 0.48, ease: "easeOut" },
    });
  }, [controls, pulseVersion, shouldReduceMotion]);

  React.useEffect(() => {
    if (shakeVersion <= previousShakeVersionRef.current) return;
    previousShakeVersionRef.current = shakeVersion;
    if (shouldReduceMotion) return;
    controls.stop();
    controls.set({ scale: 1, x: 0, y: 0 });
    void controls.start({
      x: [0, -4, 4, -3, 3, -1.5, 1.5, 0],
      transition: { duration: 0.42, ease: "easeOut" },
    });
  }, [controls, shakeVersion, shouldReduceMotion]);

  return (
    <motion.span
      animate={controls}
      className="relative block h-4.5 w-4.5 shrink-0"
      data-pulse-version={pulseVersion}
      data-shake-version={shakeVersion}
      data-testid={`composer-address-lock-${agent.pubkey}`}
      initial={false}
    >
      <UserAvatar
        avatarUrl={agent.avatarUrl}
        className="h-4.5 w-4.5"
        displayName={agent.displayName}
        shape="squircle"
        size="xs"
        testId="composer-address-lock-avatar"
      />
    </motion.span>
  );
}

function RemainingAgentCount({ count }: { count: number }) {
  return count > 0 ? (
    <span
      aria-label={`${count} more addressed ${count === 1 ? "agent" : "agents"}`}
      className="flex h-5 min-w-5 items-center justify-center rounded-full bg-muted px-1 text-3xs font-semibold text-muted-foreground ring-1 ring-border/70"
      role="img"
    >
      +{count}
    </span>
  ) : null;
}

const VISIBLE_AGENT_LIMIT = 3;

function useNewlyAddedAgentPubkeys(
  agents: readonly ComposerAddressAgent[],
): ReadonlySet<string> {
  const previousPubkeysRef = React.useRef<ReadonlySet<string> | null>(null);
  const currentPubkeys = new Set(agents.map((agent) => agent.pubkey));
  const newlyAddedPubkeys = new Set<string>();

  if (previousPubkeysRef.current) {
    for (const pubkey of currentPubkeys) {
      if (!previousPubkeysRef.current.has(pubkey)) {
        newlyAddedPubkeys.add(pubkey);
      }
    }
  }

  React.useEffect(() => {
    previousPubkeysRef.current = new Set(agents.map((agent) => agent.pubkey));
  }, [agents]);

  return newlyAddedPubkeys;
}

const addressEntryTransition = {
  type: "spring",
  stiffness: 500,
  damping: 30,
} as const;

type AddressAgentsProps = {
  agents: readonly ComposerAddressAgent[];
  pulseVersionByPubkey?: Readonly<Record<string, number>>;
  shakeVersionByPubkey?: Readonly<Record<string, number>>;
};

export function ComposerMentionButton({
  agents,
  confirmationTitle,
  disabled,
  onConfirmationDismiss,
  onConfirmationHoverChange,
  onConfirmationTurnOff,
  onCaptureSelection,
  onOpen,
  onRemove,
  pulseVersionByPubkey = {},
  shakeVersionByPubkey = {},
  showAgents,
}: AddressAgentsProps & {
  confirmationTitle?: string | null;
  disabled: boolean;
  onConfirmationDismiss?: () => void;
  onConfirmationHoverChange?: (hovered: boolean) => void;
  onConfirmationTurnOff?: () => void;
  onCaptureSelection: () => void;
  onOpen: () => void;
  onRemove: (pubkey: string) => void;
  showAgents: boolean;
}) {
  const visibleAgents = showAgents ? agents.slice(0, VISIBLE_AGENT_LIMIT) : [];
  const hiddenCount = showAgents ? agents.length - visibleAgents.length : 0;
  const hasAgents = visibleAgents.length > 0;
  const shouldReduceMotion = useReducedMotion();
  const [showActiveChrome, setShowActiveChrome] = React.useState(hasAgents);
  const newlyAddedAgentPubkeys = useNewlyAddedAgentPubkeys(visibleAgents);

  React.useEffect(() => {
    if (hasAgents) setShowActiveChrome(true);
  }, [hasAgents]);

  return (
    <Popover
      modal={false}
      onOpenChange={(open) => {
        if (!open) onConfirmationDismiss?.();
      }}
      open={Boolean(confirmationTitle)}
    >
      <PopoverAnchor asChild>
        <div
          className={cn(
            "flex h-8 min-w-8 items-center justify-center rounded-lg transition-colors",
            showActiveChrome
              ? "gap-1.5 bg-primary/15 pl-2 pr-1.5 text-primary hover:bg-primary/25 hover:text-primary/90"
              : "text-foreground",
          )}
        >
          <Tooltip disableHoverableContent>
            <TooltipTrigger asChild>
              <button
                aria-label={hasAgents ? "Manage mentions" : "Mention someone"}
                className={cn(
                  "flex h-8 items-center justify-center rounded-lg focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50",
                  showActiveChrome
                    ? "-ml-2 w-6 rounded-l-lg rounded-r-sm pl-2"
                    : "w-8 hover:bg-accent hover:text-accent-foreground",
                )}
                data-mention-picker-trigger=""
                data-testid="message-insert-mention"
                disabled={disabled}
                onClick={onOpen}
                onMouseDown={(event) => {
                  onCaptureSelection();
                  event.preventDefault();
                }}
                type="button"
              >
                <AtSign aria-hidden="true" className="h-4 w-4 shrink-0" />
              </button>
            </TooltipTrigger>
            <TooltipContent>
              {hasAgents ? "Manage mentions" : "Mention someone"}
            </TooltipContent>
          </Tooltip>
          <AnimatePresence
            initial={false}
            onExitComplete={() => {
              if (!hasAgents) setShowActiveChrome(false);
            }}
          >
            {hasAgents ? (
              <motion.span
                animate={{ opacity: 1, width: "auto" }}
                className="flex items-center gap-1 overflow-hidden"
                data-testid="composer-address-locks"
                exit={{ opacity: 0, width: 0 }}
                initial={shouldReduceMotion ? false : { opacity: 0, width: 0 }}
                transition={
                  shouldReduceMotion
                    ? { duration: 0 }
                    : { duration: 0.12, ease: "easeOut" }
                }
              >
                <AnimatePresence mode="popLayout">
                  {visibleAgents.map((agent) => (
                    <Tooltip disableHoverableContent key={agent.pubkey}>
                      <TooltipTrigger asChild>
                        <motion.button
                          aria-label={`Don't automatically mention ${agent.displayName} in this thread`}
                          animate={{ opacity: 1, scale: 1 }}
                          className="group/address relative rounded-full focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
                          data-testid={`composer-address-lock-remove-${agent.pubkey}`}
                          disabled={disabled}
                          exit={
                            shouldReduceMotion
                              ? { opacity: 0 }
                              : { opacity: 0, scale: 0.8 }
                          }
                          initial={
                            newlyAddedAgentPubkeys.has(agent.pubkey)
                              ? shouldReduceMotion
                                ? { opacity: 0 }
                                : { opacity: 0, scale: 0.8 }
                              : false
                          }
                          layout={!shouldReduceMotion}
                          onClick={() => onRemove(agent.pubkey)}
                          transition={
                            shouldReduceMotion
                              ? { duration: 0 }
                              : addressEntryTransition
                          }
                          type="button"
                        >
                          <AddressedAgentAvatar
                            agent={agent}
                            pulseVersion={
                              pulseVersionByPubkey[agent.pubkey] ?? 0
                            }
                            shakeVersion={
                              shakeVersionByPubkey[agent.pubkey] ?? 0
                            }
                          />
                          <span className="pointer-events-none absolute inset-0 flex items-center justify-center rounded-full bg-foreground text-background opacity-0 transition-opacity group-hover/address:opacity-100 group-focus-visible/address:opacity-100">
                            <X aria-hidden="true" className="h-3 w-3" />
                          </span>
                        </motion.button>
                      </TooltipTrigger>
                      <TooltipContent>
                        Don't automatically mention {agent.displayName} in this
                        thread <AgentManagementMarker pubkey={agent.pubkey} />
                      </TooltipContent>
                    </Tooltip>
                  ))}
                </AnimatePresence>
                <RemainingAgentCount count={hiddenCount} />
              </motion.span>
            ) : null}
          </AnimatePresence>
        </div>
      </PopoverAnchor>
      {confirmationTitle ? (
        <PopoverContent
          align="center"
          aria-live="polite"
          className="flex max-w-[calc(100vw-2rem)] items-center gap-2 rounded-lg px-2.5 py-1.5 text-xs"
          collisionPadding={8}
          data-testid="composer-auto-pin-confirmation"
          onCloseAutoFocus={(event) => event.preventDefault()}
          onOpenAutoFocus={(event) => event.preventDefault()}
          onPointerEnter={() => onConfirmationHoverChange?.(true)}
          onPointerLeave={() => onConfirmationHoverChange?.(false)}
          side="right"
          sideOffset={8}
          style={{ width: "max-content" }}
        >
          <span className="whitespace-nowrap">{confirmationTitle}</span>
          <button
            className="shrink-0 rounded-md px-1.5 py-1 font-medium text-primary outline-hidden hover:bg-primary/10 focus-visible:ring-1 focus-visible:ring-ring"
            onClick={onConfirmationTurnOff}
            onMouseDown={(event) => event.preventDefault()}
            type="button"
          >
            Turn off
          </button>
        </PopoverContent>
      ) : null}
    </Popover>
  );
}

export function ComposerSendButton({
  isSending,
  onFinishVoiceNote,
  sendDisabled,
}: {
  isSending: boolean;
  onFinishVoiceNote?: () => void;
  sendDisabled: boolean;
}) {
  const isFinishingVoiceNote = onFinishVoiceNote != null;
  return (
    <button
      aria-label={
        isFinishingVoiceNote
          ? "Finish voice note"
          : isSending
            ? "Sending"
            : "Send message"
      }
      className="inline-flex h-8 w-8 items-center justify-center rounded-full bg-primary text-primary-foreground shadow transition-colors hover:bg-primary/90 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
      data-testid={isFinishingVoiceNote ? "finish-voice-note" : "send-message"}
      disabled={sendDisabled || isSending}
      onClick={onFinishVoiceNote}
      type={isFinishingVoiceNote ? "button" : "submit"}
    >
      {isFinishingVoiceNote ? (
        <Square aria-hidden className="h-3.5 w-3.5 fill-current" />
      ) : isSending ? (
        <SendSpinner />
      ) : (
        <ArrowUp aria-hidden className="h-4 w-4" />
      )}
    </button>
  );
}

function SendSpinner() {
  return (
    <span
      aria-hidden
      className="h-4 w-4 animate-spin rounded-full border-2 border-primary-foreground border-t-transparent"
    />
  );
}

import { ArrowUp, AtSign, X } from "lucide-react";
import {
  AnimatePresence,
  motion,
  useAnimationControls,
  useReducedMotion,
} from "motion/react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { UserAvatar } from "@/shared/ui/UserAvatar";
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
      className="relative block h-5 w-5 shrink-0"
      data-pulse-version={pulseVersion}
      data-shake-version={shakeVersion}
      data-testid={`composer-address-lock-${agent.pubkey}`}
      initial={false}
    >
      <UserAvatar
        avatarUrl={agent.avatarUrl}
        className="h-5 w-5"
        displayName={agent.displayName}
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
  disabled,
  onCaptureSelection,
  onOpen,
  pulseVersionByPubkey = {},
  shakeVersionByPubkey = {},
  showAgents,
}: AddressAgentsProps & {
  disabled: boolean;
  onCaptureSelection: () => void;
  onOpen: () => void;
  showAgents: boolean;
}) {
  const visibleAgents = showAgents ? agents.slice(0, VISIBLE_AGENT_LIMIT) : [];
  const hiddenCount = showAgents ? agents.length - visibleAgents.length : 0;
  const hasAgents = visibleAgents.length > 0;
  const newlyAddedAgentPubkeys = useNewlyAddedAgentPubkeys(visibleAgents);

  return (
    <Tooltip disableHoverableContent>
      <TooltipTrigger asChild>
        <button
          aria-label={hasAgents ? "Manage mentioned agents" : "Mention someone"}
          className={cn(
            "flex h-8 min-w-8 items-center justify-center gap-1.5 rounded-lg transition-colors focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50",
            hasAgents
              ? "bg-primary/15 pl-2 pr-1 text-primary hover:bg-primary/25 hover:text-primary/90"
              : "text-foreground hover:bg-accent hover:text-accent-foreground",
          )}
          data-testid="message-insert-mention"
          disabled={disabled}
          onClick={onOpen}
          onMouseDown={onCaptureSelection}
          type="button"
        >
          <AtSign aria-hidden="true" className="h-4 w-4 shrink-0" />
          {hasAgents ? (
            <span
              className="flex items-center gap-1"
              data-testid="composer-address-locks"
            >
              <AnimatePresence mode="popLayout">
                {visibleAgents.map((agent) => (
                  <motion.span
                    animate={{ opacity: 1, scale: 1 }}
                    exit={{ opacity: 0, scale: 0.8 }}
                    initial={
                      newlyAddedAgentPubkeys.has(agent.pubkey)
                        ? { opacity: 0, scale: 0.8 }
                        : false
                    }
                    key={agent.pubkey}
                    layout
                    transition={addressEntryTransition}
                  >
                    <AddressedAgentAvatar
                      agent={agent}
                      pulseVersion={pulseVersionByPubkey[agent.pubkey] ?? 0}
                      shakeVersion={shakeVersionByPubkey[agent.pubkey] ?? 0}
                    />
                  </motion.span>
                ))}
              </AnimatePresence>
              <RemainingAgentCount count={hiddenCount} />
            </span>
          ) : null}
        </button>
      </TooltipTrigger>
      <TooltipContent>
        {hasAgents ? "Manage mentioned agents" : "Mention someone"}
      </TooltipContent>
    </Tooltip>
  );
}

export function ComposerSendButton({
  agents,
  isSending,
  onRemove,
  pulseVersionByPubkey = {},
  sendDisabled,
  shakeVersionByPubkey = {},
  showAgents,
}: AddressAgentsProps & {
  isSending: boolean;
  onRemove: (pubkey: string) => void;
  sendDisabled: boolean;
  showAgents: boolean;
}) {
  const visibleAgents = showAgents ? agents.slice(0, VISIBLE_AGENT_LIMIT) : [];
  const hiddenCount = showAgents ? agents.length - visibleAgents.length : 0;
  const hasAgents = visibleAgents.length > 0;
  const newlyAddedAgentPubkeys = useNewlyAddedAgentPubkeys(visibleAgents);

  if (!hasAgents) {
    return (
      <button
        aria-label={isSending ? "Sending" : "Send message"}
        className="inline-flex h-8 w-8 items-center justify-center rounded-full bg-primary text-primary-foreground shadow transition-colors hover:bg-primary/90 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
        data-testid="send-message"
        disabled={sendDisabled || isSending}
        type="submit"
      >
        {isSending ? (
          <SendSpinner />
        ) : (
          <ArrowUp aria-hidden className="h-4 w-4" />
        )}
      </button>
    );
  }

  return (
    <motion.div
      className="flex h-8 items-center gap-0.5 rounded-full bg-primary p-1 pl-1.5 text-primary-foreground shadow"
      data-testid="composer-address-send"
      layout
    >
      <span
        className="flex items-center gap-1"
        data-testid="composer-address-locks"
      >
        <AnimatePresence mode="popLayout">
          {visibleAgents.map((agent) => (
            <Tooltip disableHoverableContent key={agent.pubkey}>
              <TooltipTrigger asChild>
                <motion.button
                  aria-label={`Stop always mentioning ${agent.displayName}`}
                  animate={{ opacity: 1, scale: 1 }}
                  className="group/address relative rounded-full focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-primary-foreground"
                  data-testid={`composer-address-lock-remove-${agent.pubkey}`}
                  disabled={isSending}
                  exit={{ opacity: 0, scale: 0.8 }}
                  initial={
                    newlyAddedAgentPubkeys.has(agent.pubkey)
                      ? { opacity: 0, scale: 0.8 }
                      : false
                  }
                  layout
                  onClick={() => onRemove(agent.pubkey)}
                  transition={addressEntryTransition}
                  type="button"
                >
                  <AddressedAgentAvatar
                    agent={agent}
                    pulseVersion={pulseVersionByPubkey[agent.pubkey] ?? 0}
                    shakeVersion={shakeVersionByPubkey[agent.pubkey] ?? 0}
                  />
                  <span className="pointer-events-none absolute inset-0 flex items-center justify-center rounded-full bg-foreground/80 text-background opacity-0 transition-opacity group-hover/address:opacity-100 group-focus-visible/address:opacity-100">
                    <X aria-hidden="true" className="h-3 w-3" />
                  </span>
                </motion.button>
              </TooltipTrigger>
              <TooltipContent>
                Stop always mentioning {agent.displayName}
              </TooltipContent>
            </Tooltip>
          ))}
        </AnimatePresence>
        <RemainingAgentCount count={hiddenCount} />
      </span>
      <button
        aria-label={isSending ? "Sending" : "Send message"}
        className="flex h-6 w-7 items-center justify-center rounded-full transition-colors hover:bg-primary-foreground/10 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-primary-foreground disabled:pointer-events-none disabled:opacity-50"
        data-testid="send-message"
        disabled={sendDisabled || isSending}
        type="submit"
      >
        {isSending ? (
          <SendSpinner />
        ) : (
          <ArrowUp aria-hidden className="h-4 w-4" />
        )}
      </button>
    </motion.div>
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

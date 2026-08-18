import { AtSign, X } from "lucide-react";
import * as React from "react";
import {
  AnimatePresence,
  motion,
  useAnimationControls,
  useReducedMotion,
} from "motion/react";

import { useProfilePanel } from "@/shared/context/ProfilePanelContext";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export type ComposerAddressLock = {
  avatarUrl: string | null;
  displayName: string;
  pubkey: string;
};

function AddressMentionBadge({
  pubkey,
  pulseVersion,
}: {
  pubkey: string;
  pulseVersion: number;
}) {
  const controls = useAnimationControls();
  const shouldReduceMotion = useReducedMotion();
  const previousPulseVersionRef = React.useRef(0);

  React.useEffect(() => {
    if (pulseVersion <= previousPulseVersionRef.current) return;
    previousPulseVersionRef.current = pulseVersion;
    if (shouldReduceMotion) return;

    void controls.start({
      scale: [1, 1.35, 0.96, 1.12, 1],
      y: [0, -5, 2, -2, 0],
      transition: { duration: 0.52, ease: "easeOut" },
    });
  }, [controls, pulseVersion, shouldReduceMotion]);

  return (
    <motion.span
      animate={controls}
      aria-hidden="true"
      className="absolute -bottom-0.5 -left-0.5 flex h-4 w-4 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-sm ring-2 ring-background"
      data-pulse-version={pulseVersion}
      data-testid={`composer-address-lock-mention-badge-${pubkey}`}
      initial={false}
    >
      <AtSign className="h-2.5 w-2.5" />
    </motion.span>
  );
}

export function ComposerAddressLocks({
  agents,
  onRemove,
  pulseVersionByPubkey = {},
}: {
  agents: readonly ComposerAddressLock[];
  onRemove: (pubkey: string) => void;
  pulseVersionByPubkey?: Readonly<Record<string, number>>;
}) {
  const { openProfilePanel } = useProfilePanel();

  return (
    <div
      className="flex min-w-0 select-none flex-wrap items-center gap-1.5"
      data-testid="composer-address-locks"
    >
      <AnimatePresence initial={false} mode="popLayout">
        {agents.map((agent) => (
          <motion.div
            animate={{ opacity: 1, scale: 1 }}
            className="group relative h-9 w-9 shrink-0"
            data-testid={`composer-address-lock-${agent.pubkey}`}
            exit={{ opacity: 0, scale: 0.8 }}
            initial={{ opacity: 0, scale: 0.8 }}
            key={agent.pubkey}
            layout
            transition={{ type: "spring", stiffness: 500, damping: 30 }}
          >
            <button
              aria-label={`Open profile for ${agent.displayName}`}
              className="relative h-9 w-9 rounded-full focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
              onClick={() => openProfilePanel?.(agent.pubkey)}
              type="button"
            >
              <UserAvatar
                avatarUrl={agent.avatarUrl}
                className="h-8 w-8 ring-1 ring-border/70"
                displayName={agent.displayName}
                size="md"
                testId="composer-address-lock-avatar"
              />
              <AddressMentionBadge
                pubkey={agent.pubkey}
                pulseVersion={pulseVersionByPubkey[agent.pubkey] ?? 0}
              />
            </button>
            <Tooltip disableHoverableContent>
              <TooltipTrigger asChild>
                <button
                  aria-label={`Stop always mentioning ${agent.displayName}`}
                  className="pointer-events-none absolute -right-0.5 -top-0.5 z-10 flex h-4 w-4 items-center justify-center rounded-full bg-foreground text-background opacity-0 ring-2 ring-background transition-opacity group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100 focus-visible:pointer-events-auto focus-visible:opacity-100"
                  data-testid={`composer-address-lock-remove-${agent.pubkey}`}
                  onClick={() => onRemove(agent.pubkey)}
                  type="button"
                >
                  <X className="h-2.5 w-2.5" />
                </button>
              </TooltipTrigger>
              <TooltipContent align="start" alignOffset={-22}>
                Stop always mentioning {agent.displayName}
              </TooltipContent>
            </Tooltip>
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
}

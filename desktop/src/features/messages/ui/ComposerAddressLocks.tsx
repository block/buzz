import { AtSign } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";

import { UserAvatar } from "@/shared/ui/UserAvatar";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export type ComposerAddressLock = {
  avatarUrl: string | null;
  displayName: string;
  pubkey: string;
};

export function ComposerAddressLocks({
  agents,
  onRemove,
}: {
  agents: readonly ComposerAddressLock[];
  onRemove: (pubkey: string) => void;
}) {
  return (
    <div
      className="flex min-w-0 select-none flex-wrap items-center gap-1.5"
      data-testid="composer-address-locks"
    >
      <AnimatePresence initial={false} mode="popLayout">
        {agents.map((agent) => (
          <motion.div
            animate={{ opacity: 1, scale: 1 }}
            className="relative h-10 w-10 shrink-0"
            data-testid={`composer-address-lock-${agent.pubkey}`}
            exit={{ opacity: 0, scale: 0.8 }}
            initial={{ opacity: 0, scale: 0.8 }}
            key={agent.pubkey}
            layout
            transition={{ type: "spring", stiffness: 500, damping: 30 }}
          >
            <Tooltip disableHoverableContent>
              <TooltipTrigger asChild>
                <button
                  aria-label={`Stop always mentioning ${agent.displayName}`}
                  aria-pressed="true"
                  className="relative h-10 w-10 rounded-full focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
                  onClick={() => onRemove(agent.pubkey)}
                  type="button"
                >
                  <UserAvatar
                    avatarUrl={agent.avatarUrl}
                    className="ring-1 ring-border/70"
                    displayName={agent.displayName}
                    size="md"
                    testId="composer-address-lock-avatar"
                  />
                  <span
                    aria-hidden="true"
                    className="absolute bottom-0 right-0 flex h-5 w-5 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-sm ring-2 ring-background"
                  >
                    <AtSign className="h-3 w-3" />
                  </span>
                </button>
              </TooltipTrigger>
              <TooltipContent className="select-none">
                Always mentioning {agent.displayName}
              </TooltipContent>
            </Tooltip>
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
}

import { AtSign, LockOpen } from "lucide-react";
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
                  aria-label={`Always mention ${agent.displayName}`}
                  aria-pressed="true"
                  className="group/address-lock relative h-10 w-10 rounded-full focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
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
                    className="absolute bottom-0 right-0 h-5 w-5 overflow-hidden rounded-full bg-primary text-primary-foreground shadow-sm ring-2 ring-background"
                  >
                    <span className="absolute inset-y-0 left-0 flex w-10 transition-transform duration-200 ease-out motion-reduce:transition-none group-hover/address-lock:-translate-x-5 group-focus-visible/address-lock:-translate-x-5">
                      <span className="flex h-5 w-5 shrink-0 items-center justify-center">
                        <AtSign className="h-3 w-3" />
                      </span>
                      <span className="flex h-5 w-5 shrink-0 items-center justify-center">
                        <LockOpen className="h-3 w-3" />
                      </span>
                    </span>
                  </span>
                </button>
              </TooltipTrigger>
              <TooltipContent className="select-none">
                Always mention {agent.displayName}
              </TooltipContent>
            </Tooltip>
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
}

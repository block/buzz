import type * as React from "react";
import { Hash } from "lucide-react";

import { cn } from "@/shared/lib/cn";

export type ChannelIntroAction = {
  description?: string;
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  testId?: string;
};

export type ChannelIntro = {
  actions?: ChannelIntroAction[];
  channelKindLabel: string;
  channelName: string;
  description?: string | null;
  icon?: React.ReactNode;
};

/**
 * The empty-channel intro block: channel icon, heading, kind label, and
 * action cards. Rendered both as the virtualized timeline's leading row and
 * as the non-virtualized empty state — one component so the two surfaces
 * cannot drift and the first message always lands below it without layout
 * shift.
 */
export function ChannelIntroBlock({
  className,
  intro,
}: {
  className?: string;
  intro: ChannelIntro;
}) {
  return (
    <div
      className={cn(
        "flex w-full flex-col items-start px-3 text-left",
        className,
      )}
      data-testid="message-channel-intro"
    >
      <div
        className="flex h-[60px] w-[60px] items-center justify-center rounded-2xl border border-border/70 bg-muted/40 text-muted-foreground"
        data-testid="message-channel-intro-icon"
      >
        {intro.icon ?? <Hash aria-hidden className="h-7 w-7" />}
      </div>
      <p className="mt-4 max-w-2xl truncate text-xl font-semibold leading-7 tracking-tight text-foreground">
        #{intro.channelName}
      </p>
      <p className="mt-1 max-w-2xl text-sm leading-5 text-muted-foreground">
        This is the beginning of the{" "}
        <span className="font-medium text-foreground">
          {intro.channelKindLabel}
        </span>
        .
      </p>
      {intro.description ? (
        <p className="mt-2 max-w-xl text-sm leading-5 text-muted-foreground">
          {intro.description}
        </p>
      ) : null}
      {intro.actions?.length ? (
        <div className="mt-4 flex max-w-full flex-nowrap gap-3 overflow-x-auto pb-1">
          {intro.actions.map((action) => {
            const hasDescription = Boolean(action.description);

            return (
              <button
                className={cn(
                  "flex shrink-0 border border-border/70 bg-background/70 text-left transition-colors hover:bg-muted/60 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
                  hasDescription
                    ? "h-56 w-[13.75rem] flex-col rounded-2xl p-4"
                    : "h-28 w-64 flex-col rounded-2xl p-4",
                )}
                data-testid={action.testId}
                key={action.label}
                onClick={action.onClick}
                type="button"
              >
                <span
                  className={cn(
                    "flex shrink-0 items-center justify-center rounded-full bg-muted/70 text-muted-foreground",
                    hasDescription
                      ? "h-12 w-12 [&_svg]:h-6 [&_svg]:w-6"
                      : "h-10 w-10 [&_svg]:h-4 [&_svg]:w-4",
                  )}
                  data-testid={
                    action.testId ? `${action.testId}-icon` : undefined
                  }
                >
                  {action.icon}
                </span>
                <span className="mt-auto min-w-0">
                  <span
                    className="block whitespace-normal break-words text-base font-medium leading-6 text-foreground"
                    data-testid={
                      action.testId ? `${action.testId}-title` : undefined
                    }
                  >
                    {action.label}
                  </span>
                  {action.description ? (
                    <span
                      className="mt-1 block whitespace-normal break-words text-sm leading-5 text-muted-foreground"
                      data-testid={
                        action.testId
                          ? `${action.testId}-description`
                          : undefined
                      }
                    >
                      {action.description}
                    </span>
                  ) : null}
                </span>
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

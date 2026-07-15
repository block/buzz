import { Hash, Inbox, MessageSquare } from "lucide-react";

import { BeeMark } from "@/features/onboarding/marketing/BeeMark";

/**
 * First-run welcome empty state (onboarding step 7a).
 *
 * Shown in the home main area the first time a freshly-onboarded user lands in
 * the app. Reuses the marketing bee mark and offers three getting-started
 * actions. Persists until dismissed / first activity (per product decision).
 */
export type WelcomeAction = "inbox" | "dm" | "channels";

export function WelcomeEmptyState({
  onAction,
}: {
  onAction: (action: WelcomeAction) => void;
}) {
  return (
    <div
      className="flex h-full min-h-0 flex-1 items-center justify-center overflow-y-auto px-6 py-10"
      data-testid="home-welcome-empty-state"
    >
      <div className="flex w-full max-w-[540px] flex-col items-center text-center">
        <BeeMark className="mb-6 w-20 [&_rect]:fill-primary" />

        <h1 className="text-3xl font-semibold tracking-tight text-foreground">
          Welcome to Buzz
        </h1>
        <p className="mt-3 text-base leading-6 text-muted-foreground">
          You're all set. Check your inbox, start a conversation, or browse
          channels to get going.
        </p>

        <div className="mt-8 grid w-full grid-cols-1 gap-3 sm:grid-cols-3">
          <WelcomeActionCard
            icon={<Inbox className="h-5 w-5" />}
            label="Check your inbox"
            testId="home-welcome-action-inbox"
            onClick={() => onAction("inbox")}
          />
          <WelcomeActionCard
            icon={<MessageSquare className="h-5 w-5" />}
            label="Start a conversation"
            testId="home-welcome-action-dm"
            onClick={() => onAction("dm")}
          />
          <WelcomeActionCard
            icon={<Hash className="h-5 w-5" />}
            label="Browse channels"
            testId="home-welcome-action-channels"
            onClick={() => onAction("channels")}
          />
        </div>
      </div>
    </div>
  );
}

function WelcomeActionCard({
  icon,
  label,
  testId,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  testId: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      data-testid={testId}
      onClick={onClick}
      className="flex flex-col items-center gap-2 rounded-xl border border-border/60 bg-card p-4 text-center transition-all hover:border-primary/50 hover:bg-muted/40 hover:shadow-md"
    >
      <span className="flex h-10 w-10 items-center justify-center rounded-full bg-primary/15 text-primary">
        {icon}
      </span>
      <span className="text-sm font-medium text-foreground">{label}</span>
    </button>
  );
}

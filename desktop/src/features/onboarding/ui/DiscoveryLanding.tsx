import {
  FEATURED_COMMUNITIES,
  type FeaturedCommunity,
} from "@/features/onboarding/featuredCommunities";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";

import { ONBOARDING_SECONDARY_CTA_CLASS } from "./OnboardingChrome";

/**
 * EXPLORATION — Discord-style first-open landing.
 *
 * The very first screen a fresh install shows: a directory of communities
 * the user can join right away. No identity ceremony, no agent corridor —
 * clicking Join creates the key silently and connects. Agent setup and key
 * backup become one avenue off this screen (`onAdvancedSetup`) instead of
 * the mandatory path.
 */
export function DiscoveryLanding({
  error,
  isPending,
  onAdvancedSetup,
  onImportKey,
  onJoin,
}: {
  error: string | null;
  isPending: boolean;
  /** Classic corridor: identity → backup → harness → config. */
  onAdvancedSetup: () => void;
  onImportKey: () => void;
  onJoin: (community: FeaturedCommunity) => void;
}) {
  return (
    <div
      className="flex w-full max-w-[860px] flex-col items-center text-center"
      data-testid="discovery-landing"
    >
      <img
        alt="Buzz"
        className="w-full max-w-[420px]"
        src="/landing/buzz-wordmark.png"
      />
      <h1 className="mt-4 text-2xl font-normal leading-tight text-foreground">
        Find your people
      </h1>
      <p className="mt-2 max-w-[520px] text-sm leading-6 text-foreground/80">
        Jump into a community — we’ll set up your identity as you go. Your
        agents, backups, and settings are one click away once you’re in.
      </p>
      {error ? <p className="mt-4 text-sm text-destructive">{error}</p> : null}
      <div className="mt-10 grid w-full grid-cols-1 gap-x-10 gap-y-12 sm:grid-cols-2">
        {FEATURED_COMMUNITIES.map((community) => (
          <Card
            className="items-stretch px-7 py-5 text-left [--buzz-card-textured-min-height:132px]"
            key={community.id}
            variant="textured"
          >
            <div className="flex items-start gap-4">
              <span
                aria-hidden
                className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-foreground/8 text-2xl"
              >
                {community.emoji}
              </span>
              <div className="min-w-0 flex-1">
                <div className="flex items-baseline justify-between gap-3">
                  <span className="truncate text-base font-medium text-foreground">
                    {community.name}
                  </span>
                  <span className="shrink-0 text-xs text-foreground/60">
                    {community.members.toLocaleString()} members
                  </span>
                </div>
                <p className="mt-1 line-clamp-2 text-sm leading-5 text-foreground/75">
                  {community.tagline}
                </p>
              </div>
            </div>
            <div className="mt-4 flex justify-end">
              <Button
                className="h-8 rounded-full px-5"
                data-testid={`discovery-join-${community.id}`}
                disabled={isPending}
                onClick={() => onJoin(community)}
                size="sm"
                type="button"
              >
                Join
              </Button>
            </div>
          </Card>
        ))}
      </div>
      <div className="mt-12 flex flex-col items-center gap-3 pb-10">
        <p className="text-sm text-foreground/70">
          Have an invite link, or want to run your own?
        </p>
        <div className="flex flex-wrap items-center justify-center gap-3">
          <Button
            className={ONBOARDING_SECONDARY_CTA_CLASS}
            data-testid="discovery-advanced-setup"
            disabled={isPending}
            onClick={onAdvancedSetup}
            type="button"
            variant="ghost"
          >
            Set up identity &amp; agents first
          </Button>
          <Button
            className={ONBOARDING_SECONDARY_CTA_CLASS}
            data-testid="discovery-import-key"
            disabled={isPending}
            onClick={onImportKey}
            type="button"
            variant="ghost"
          >
            I already have a key
          </Button>
        </div>
      </div>
    </div>
  );
}

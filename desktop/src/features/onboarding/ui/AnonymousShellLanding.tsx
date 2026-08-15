import { Compass, Plus } from "lucide-react";

import {
  FEATURED_COMMUNITIES,
  type FeaturedCommunity,
} from "@/features/onboarding/featuredCommunities";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { Skeleton } from "@/shared/ui/skeleton";

import { ONBOARDING_SECONDARY_CTA_CLASS } from "./OnboardingChrome";

/**
 * EXPLORATION — anonymous-first app shell (Discord model).
 *
 * A fresh install opens straight into something that looks like the app:
 * a ghost sidebar on the left and a community-discovery home in the main
 * pane. No identity ceremony and no agent greeting the user — joining a
 * community creates the key silently, and agents become something you
 * discover later. The classic corridor and key import stay reachable as
 * quiet options below the fold.
 */
export function AnonymousShellLanding({
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
    <div className="flex h-full w-full" data-testid="discovery-landing">
      <aside
        aria-hidden
        className="flex w-14 shrink-0 flex-col items-center gap-2.5 border-r border-sidebar-border/60 bg-sidebar px-2.5 pb-5 pt-12"
        data-testid="anon-shell-rail"
      >
        <span className="flex h-9 w-9 items-center justify-center rounded-xl bg-primary text-primary-foreground">
          <Compass className="h-4 w-4" />
        </span>
        <span className="my-0.5 h-px w-7 bg-sidebar-border" />
        <Skeleton className="h-9 w-9 rounded-2xl" pulsing={false} />
        <Skeleton className="h-9 w-9 rounded-2xl" pulsing={false} />
        <span className="flex h-9 w-9 items-center justify-center rounded-2xl bg-sidebar-accent/60 text-sidebar-foreground/70">
          <Plus className="h-4 w-4" />
        </span>
      </aside>
      <aside
        aria-hidden
        className="flex w-60 shrink-0 flex-col border-r border-sidebar-border bg-sidebar px-3 pb-3 pt-12"
        data-testid="anon-shell-sidebar"
      >
        <div className="rounded-lg bg-foreground/8 px-3 py-2 text-xs text-foreground/45">
          Search everything
        </div>
        <div className="mt-4 flex flex-col gap-0.5">
          {["Inbox", "Pulse", "Projects", "Agents"].map((label) => (
            <div
              className="rounded-md px-2 py-1.5 text-sm text-foreground/40"
              key={label}
            >
              {label}
            </div>
          ))}
        </div>
        <div className="mt-7 px-2 text-xs font-medium uppercase tracking-wide text-foreground/35">
          Channels
        </div>
        <div className="mt-3 flex flex-col gap-3 px-2">
          <Skeleton className="h-3.5 w-32" pulsing={false} />
          <Skeleton className="h-3.5 w-24" pulsing={false} />
          <Skeleton className="h-3.5 w-28" pulsing={false} />
        </div>
        <div className="mt-8 px-2 text-xs font-medium uppercase tracking-wide text-foreground/35">
          Direct messages
        </div>
        <div className="mt-3 flex flex-col gap-3 px-2">
          <Skeleton className="h-3.5 w-28" pulsing={false} />
          <Skeleton className="h-3.5 w-20" pulsing={false} />
        </div>
        <div className="mt-auto rounded-xl bg-foreground/5 px-3 py-2.5 text-left">
          <p className="text-sm font-medium text-foreground/80">Guest</p>
          <p className="mt-0.5 text-xs leading-4 text-foreground/50">
            Join a community to claim your identity
          </p>
        </div>
      </aside>
      <main className="flex min-w-0 flex-1 flex-col overflow-y-auto">
        <div className="mx-auto w-full max-w-[880px] px-8 pb-12 pt-14">
          <div className="rounded-2xl bg-foreground/5 px-10 py-12 text-center">
            <h1 className="text-3xl font-semibold leading-tight text-foreground">
              Find your community on Buzz
            </h1>
            <p className="mx-auto mt-3 max-w-[480px] text-sm leading-6 text-foreground/70">
              From building agents to protocol talk, there’s a place for you.
              Jump in — your identity is created as you go.
            </p>
          </div>
          {error ? (
            <p className="mt-6 text-center text-sm text-destructive">{error}</p>
          ) : null}
          <h2 className="mt-10 text-left text-base font-medium text-foreground/85">
            Featured communities
          </h2>
          <div className="mt-4 grid w-full grid-cols-1 gap-x-8 gap-y-10 sm:grid-cols-2">
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
          <div className="mt-12 flex flex-col items-center gap-3">
            <p className="text-sm text-foreground/70">
              Want to set things up yourself first?
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
      </main>
    </div>
  );
}

import { openUrl } from "@tauri-apps/plugin-opener";
import { ArrowRight, Link2, Plus, Settings2, Trash2 } from "lucide-react";
import * as React from "react";

import { ThemeGrainientBackground } from "@/app/ThemeGrainientBackground";
import type { Community } from "@/features/communities/types";
import { useCommunityIcons } from "@/features/communities/useCommunityIcons";
import { getInitials } from "@/shared/lib/initials";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";

const CREATE_COMMUNITY_URL = "https://app.builderlab.xyz/signup?returnTo=/buzz";
const HEX_CLIP_PATH =
  "polygon(25% 6.7%, 75% 6.7%, 100% 50%, 75% 93.3%, 25% 93.3%, 0 50%)";

function CommunityHex({
  community,
  iconUrl,
  onOpen,
  onRemove,
}: {
  community: Community;
  iconUrl: string | null;
  onOpen: () => void;
  onRemove: () => void;
}) {
  return (
    <ContextMenu modal={false}>
      <ContextMenuTrigger asChild>
        <button
          aria-label={`Open ${community.name}`}
          className="group relative aspect-[1.06] w-full outline-hidden transition-transform duration-300 hover:-translate-y-1 focus-visible:-translate-y-1"
          data-testid={`community-home-community-${community.id}`}
          onClick={onOpen}
          type="button"
        >
          <span
            aria-hidden="true"
            className="absolute inset-0 bg-foreground/12 transition-colors group-hover:bg-foreground/25 group-focus-visible:bg-foreground/30"
            style={{ clipPath: HEX_CLIP_PATH }}
          />
          <span
            className="absolute inset-px flex flex-col items-center justify-center overflow-hidden bg-card/92 px-[14%] text-card-foreground shadow-xl backdrop-blur-md transition-colors group-hover:bg-card"
            style={{ clipPath: HEX_CLIP_PATH }}
          >
            <span className="absolute inset-0 bg-[radial-gradient(circle_at_36%_24%,hsl(var(--primary)/0.18),transparent_58%)]" />
            <span className="relative flex h-[38%] w-[38%] items-center justify-center overflow-hidden rounded-[28%] bg-primary/12 text-title font-semibold text-primary shadow-sm ring-1 ring-primary/15 transition-transform duration-300 group-hover:scale-105">
              {iconUrl ? (
                <img
                  alt=""
                  className="h-full w-full object-cover"
                  draggable={false}
                  src={iconUrl}
                />
              ) : (
                getInitials(community.name) || "🐝"
              )}
            </span>
            <span className="relative mt-3 max-w-full truncate text-sm font-medium">
              {community.name}
            </span>
            <span className="relative mt-1 flex items-center gap-1 text-xs text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100">
              Enter <ArrowRight className="h-3 w-3" />
            </span>
          </span>
        </button>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onClick={onOpen}>
          <ArrowRight className="h-4 w-4" />
          Open community
        </ContextMenuItem>
        <ContextMenuItem
          onClick={() => void navigator.clipboard.writeText(community.relayUrl)}
        >
          <Link2 className="h-4 w-4" />
          Copy relay URL
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem className="text-destructive" onClick={onRemove}>
          <Trash2 className="h-4 w-4" />
          Remove from Buzz
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

function ActionHex({
  label,
  detail,
  icon,
  onClick,
  testId,
}: {
  label: string;
  detail: string;
  icon: React.ReactNode;
  onClick: () => void;
  testId: string;
}) {
  return (
    <button
      className="group relative aspect-[1.06] w-full outline-hidden transition-transform duration-300 hover:-translate-y-1 focus-visible:-translate-y-1"
      data-testid={testId}
      onClick={onClick}
      type="button"
    >
      <span
        className="absolute inset-0 bg-primary/35 transition-colors group-hover:bg-primary/65"
        style={{ clipPath: HEX_CLIP_PATH }}
      />
      <span
        className="absolute inset-px flex flex-col items-center justify-center bg-primary/8 px-[14%] text-foreground backdrop-blur-md transition-colors group-hover:bg-primary/14"
        style={{ clipPath: HEX_CLIP_PATH }}
      >
        <span className="flex h-[34%] w-[34%] items-center justify-center rounded-full bg-primary text-primary-foreground shadow-lg transition-transform duration-300 group-hover:scale-105">
          {icon}
        </span>
        <span className="mt-3 text-sm font-medium">{label}</span>
        <span className="mt-1 text-xs text-muted-foreground">{detail}</span>
      </span>
    </button>
  );
}

export function CommunityHome({
  communities,
  onOpenCommunity,
  onJoinCommunity,
  onRemoveCommunity,
  onBackToMachineConfig,
}: {
  communities: Community[];
  onOpenCommunity: (id: string) => void;
  onJoinCommunity: () => void;
  onRemoveCommunity: (id: string) => void;
  onBackToMachineConfig: () => void;
}) {
  const iconsByCommunity = useCommunityIcons(communities);
  const [communityToRemove, setCommunityToRemove] =
    React.useState<Community | null>(null);

  return (
    <main
      className="relative min-h-dvh overflow-y-auto bg-background text-foreground"
      data-testid="community-home"
    >
      <StartupWindowDragRegion />
      <ThemeGrainientBackground />
      <div className="pointer-events-none absolute inset-0 bg-background/35 backdrop-blur-3xl" />
      <div className="relative mx-auto flex min-h-dvh w-full max-w-6xl flex-col px-8 pb-16 pt-20 sm:px-12 lg:px-16">
        <header className="flex items-start justify-between gap-8">
          <div>
            <p className="text-xs font-medium uppercase tracking-[0.22em] text-muted-foreground">
              Buzz communities
            </p>
            <h1 className="mt-3 text-title font-normal">
              Where do you want to go?
            </h1>
            <p className="mt-3 max-w-xl text-sm leading-6 text-muted-foreground">
              Your communities, all in one place. Choose one to enter or make a
              new space for your people.
            </p>
          </div>
          <Button
            aria-label="Identity settings"
            className="mt-1 rounded-full bg-background/50 backdrop-blur-md"
            onClick={onBackToMachineConfig}
            size="icon"
            type="button"
            variant="outline"
          >
            <Settings2 className="h-4 w-4" />
          </Button>
        </header>

        <section
          aria-label="Communities"
          className={cn(
            "mx-auto mt-12 grid w-full max-w-4xl grid-cols-2 gap-x-3 gap-y-0 sm:grid-cols-3 md:grid-cols-4",
            "[&>*:nth-child(even)]:translate-y-[45%] sm:[&>*:nth-child(even)]:translate-y-0",
            "sm:[&>*:nth-child(3n+2)]:translate-y-[45%] md:[&>*:nth-child(3n+2)]:translate-y-0",
            "md:[&>*:nth-child(even)]:translate-y-[45%]",
          )}
        >
          {communities.map((community) => (
            <CommunityHex
              community={community}
              iconUrl={iconsByCommunity[community.id] ?? null}
              key={community.id}
              onOpen={() => onOpenCommunity(community.id)}
              onRemove={() => setCommunityToRemove(community)}
            />
          ))}
          <ActionHex
            detail="Use a relay URL"
            icon={<Plus className="h-6 w-6" />}
            label="Join a community"
            onClick={onJoinCommunity}
            testId="community-home-join"
          />
          <ActionHex
            detail="Start something new"
            icon={<span className="text-xl">✦</span>}
            label="Create a community"
            onClick={() => void openUrl(CREATE_COMMUNITY_URL)}
            testId="community-home-create"
          />
        </section>

        {communities.length === 0 ? (
          <p className="mx-auto mt-24 max-w-md text-center text-sm leading-6 text-muted-foreground sm:mt-28">
            This is your community home. It will stay available whenever you
            want a neutral place to join, create, or switch communities.
          </p>
        ) : null}
      </div>
      <AlertDialog
        onOpenChange={(open) => {
          if (!open) setCommunityToRemove(null);
        }}
        open={communityToRemove !== null}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Remove {communityToRemove?.name ?? "community"} from Buzz?
            </AlertDialogTitle>
            <AlertDialogDescription>
              This removes the saved community from this device. It does not
              delete the community or your membership.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel asChild>
              <Button type="button" variant="outline">
                Cancel
              </Button>
            </AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button
                onClick={() => {
                  if (communityToRemove) {
                    onRemoveCommunity(communityToRemove.id);
                  }
                  setCommunityToRemove(null);
                }}
                type="button"
                variant="destructive"
              >
                Remove community
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </main>
  );
}

import { Waypoints } from "lucide-react";

import { SidebarMenuButton, SidebarMenuItem } from "@/shared/ui/sidebar";
import { SidebarMenuLabel } from "@/shared/ui/sidebar-menu-label";
import { Switch } from "@/shared/ui/switch";
import { cn } from "@/shared/lib/cn";

import { useMeshComputeState } from "../hooks/useMeshComputeState";
import type { MeshRowPulse, MeshRowTone } from "../meshRowModel";

/**
 * The sidebar Shared Compute row.
 *
 * Shaped like Inbox and Agents: a subject icon on the left, the name beside it.
 * The status signal rides next to the name rather than standing in for the icon
 * — an earlier revision made the dot the leading element, which read as a bullet
 * point instead of a peer of the other rows.
 *
 * The dot distinguishes six states, and the pair that matters most is `unknown`
 * vs `available`: "nobody is sharing" and "there is 115 GB here you have not
 * joined" must not look alike, since the second is the reason to look at all.
 * See `meshRowModel.ts`.
 */

const TONE_DOT: Record<MeshRowTone, string> = {
  // Hollow: nothing known. Deliberately not a filled grey, which would read as
  // a state rather than an absence of information.
  unknown: "border border-muted-foreground/45 bg-transparent",
  available: "bg-slate-400 dark:bg-slate-500",
  sharing: "bg-emerald-500 dark:bg-emerald-400",
  consuming: "bg-sky-500 dark:bg-sky-400",
  starting: "bg-amber-500 dark:bg-amber-400",
  failed: "bg-transparent border-2 border-destructive",
};

/**
 * How the dot moves. Brightness only.
 *
 * No scaling: a dot that grows shifts the row's optical baseline and reads as
 * jitter rather than as a signal. Opacity is what Tailwind's `pulse` animates and
 * what Buzz already uses to mean "working" (`ChannelWorkingBadge`,
 * `AgentStatusBadge`).
 *
 * Both kinds share one curve and differ only in tempo, because the difference
 * between "happening now" and "in progress" is urgency, not character. There is
 * no third kind: standing states do not animate at all.
 *
 * (An earlier revision animated a separate halo ring, scaling it to 2x while
 * fading to zero. At 8px that is imperceptible, which is why the pulse could not
 * be seen. Then it over-corrected into scaling the dot, which jittered.)
 */
const DOT_PULSE_CLASS: Record<MeshRowPulse, string> = {
  none: "",
  activity: "animate-mesh-activity",
  progress: "animate-mesh-progress",
};

/** The status dot. Shared by both placements. */
function MeshPulseDot({
  pulse,
  tone,
  size = "default",
  testId,
}: {
  pulse: MeshRowPulse;
  tone: MeshRowTone;
  /** `small` is the icon-collapsed variant, which has to fit on the glyph. */
  size?: "default" | "small";
  testId?: string;
}) {
  const box = size === "small" ? "h-1.5 w-1.5" : "h-2 w-2";
  return (
    <span
      aria-hidden
      className={cn("relative flex shrink-0 items-center justify-center", box)}
    >
      <span
        className={cn(
          "rounded-full motion-reduce:animate-none",
          box,
          TONE_DOT[tone],
          DOT_PULSE_CLASS[pulse],
        )}
        data-mesh-pulse={pulse}
        data-testid={testId}
      />
    </span>
  );
}

export function SidebarMeshComputeRow({
  onOpenComputeSettings,
}: {
  onOpenComputeSettings?: () => void;
}) {
  const mesh = useMeshComputeState();
  const { row, toggle, pendingAction, shareSwitchChecked, setSharing } = mesh;

  // Clicking the row is navigation: Shared Compute opens the community Compute
  // view. The trailing switch remains a separate action and does not navigate.
  const showJoinToggle = !toggle.isSharing;

  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        className={cn(
          "data-[active=true]:font-normal",
          // Reserve the trailing slot so the label truncates before it runs
          // under the switch, rather than colliding with it.
          showJoinToggle && "pr-11 group-data-[collapsible=icon]:!pr-0",
        )}
        data-mesh-tone={row.tone}
        data-testid="sidebar-mesh-compute-row"
        onClick={onOpenComputeSettings}
        tooltip={row.tooltip}
        type="button"
      >
        <span
          aria-hidden
          className="relative flex h-4 w-4 shrink-0 items-center justify-center"
        >
          <Waypoints className="h-4 w-4 opacity-80" />
          {/*
              Icon-collapsed only. The label and the dot beside it are both
              clipped in icon mode, so without this the row would lose its status
              signal entirely at exactly the width where the tooltip is the only
              other affordance.
            */}
          <span className="absolute -right-1 -bottom-1 hidden rounded-full bg-sidebar p-px group-data-[collapsible=icon]:block">
            <MeshPulseDot pulse={row.pulse} size="small" tone={row.tone} />
          </span>
        </span>
        <SidebarMenuLabel className="opacity-80">{row.label}</SidebarMenuLabel>
        {/*
            Right-aligned, the same position and mechanism a channel row uses for
            its unread dot (`UnreadDotBadge className="ml-auto"`). Beside the name
            it competed with the label for one optical line; here it reads as this
            row's indicator and matches the sidebar's existing vocabulary.

            When the join switch is present the button carries `pr-11`, so the dot
            lands clear of it rather than underneath.
          */}
        <span className="ml-auto group-data-[collapsible=icon]:hidden">
          <MeshPulseDot
            pulse={row.pulse}
            testId="mesh-row-dot"
            tone={row.tone}
          />
        </span>
      </SidebarMenuButton>

      {/*
        A sibling of the navigation button, never a child: nesting this switch
        would make toggling sharing navigate away from the current view.
      */}
      {showJoinToggle ? (
        <div className="absolute top-1/2 right-2 -translate-y-1/2 group-data-[collapsible=icon]:hidden">
          <Switch
            aria-label="Share this computer's compute"
            // Shared with the popover switch so the two controls for one action
            // cannot disagree. Optimistic by design -- see `shareSwitchChecked`.
            checked={shareSwitchChecked}
            className={cn(
              "h-4 w-7 [&>span]:h-3 [&>span]:w-3 [&>span]:data-[state=checked]:translate-x-3",
              "data-[state=unchecked]:bg-foreground/25",
            )}
            data-testid="mesh-row-share-toggle"
            disabled={pendingAction !== null}
            onCheckedChange={setSharing}
          />
        </div>
      ) : null}
    </SidebarMenuItem>
  );
}

import {
  Anchor,
  Bell,
  ClipboardList,
  Radar,
  Route,
  type LucideIcon,
} from "lucide-react";

import sextantInsignia from "@/assets/command-adviser/sextant-insignia.png";
import { cn } from "@/shared/lib/cn";

export type CommandAdviserId =
  | "chief_of_staff"
  | "operations"
  | "navigation"
  | "daily_routine"
  | "reporting"
  | "plans";

type AdviserIdentity = {
  readonly label: string;
  readonly symbol: string;
  readonly icon?: LucideIcon;
};

export const ADVISER_IDENTITIES: Record<CommandAdviserId, AdviserIdentity> = {
  chief_of_staff: {
    label: "Chief of Staff — command anchor",
    symbol: "command-anchor",
    icon: Anchor,
  },
  operations: {
    label: "Operations Adviser — radar plot",
    symbol: "radar-plot",
    icon: Radar,
  },
  navigation: {
    label: "Navigation Adviser — sextant",
    symbol: "sextant",
  },
  daily_routine: {
    label: "Daily Routine Adviser — ship's bell",
    symbol: "ships-bell",
    icon: Bell,
  },
  reporting: {
    label: "Reporting Adviser — clipboard and returns",
    symbol: "clipboard-returns",
    icon: ClipboardList,
  },
  plans: {
    label: "Plans Adviser — charted course",
    symbol: "charted-course",
    icon: Route,
  },
};

function testId(adviser: CommandAdviserId) {
  return adviser.replaceAll("_", "-");
}

export function AdviserInsignia({
  adviser,
  className,
}: {
  adviser: CommandAdviserId;
  className?: string;
}) {
  const identity = ADVISER_IDENTITIES[adviser];
  const Icon = identity.icon;

  return (
    <div
      aria-label={identity.label}
      className={cn(
        "flex h-12 w-12 shrink-0 items-center justify-center overflow-hidden rounded-full border border-[#d8aa4f]/50 bg-[#071a2f] text-[#e5bd65] shadow-sm",
        className,
      )}
      data-symbol={identity.symbol}
      data-testid={`adviser-insignia-${testId(adviser)}`}
      role="img"
    >
      {Icon ? (
        <Icon aria-hidden="true" className="h-6 w-6" strokeWidth={1.75} />
      ) : (
        <img
          alt=""
          className="h-full w-full object-cover"
          src={sextantInsignia}
        />
      )}
    </div>
  );
}

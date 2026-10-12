// Phone-only bottom tab bar (rendered only when usePhoneLayout() is true). iPad/desktop never mount it.
import { Bot, CircleUser, Inbox, MessagesSquare, SquareTerminal } from "lucide-react";
import type * as React from "react";

import { cn } from "@/shared/lib/cn";
import type { PhoneTab } from "./phoneTabs";

const TABS: { id: PhoneTab; label: string; Icon: React.ComponentType<{ className?: string }> }[] = [
  { id: "inbox", label: "Inbox", Icon: Inbox },
  { id: "channels", label: "Channels", Icon: MessagesSquare },
  { id: "agents", label: "Agents", Icon: Bot },
  { id: "term", label: "Buzz Term", Icon: SquareTerminal },
  { id: "me", label: "Me", Icon: CircleUser },
];

function Badge({ count }: { count: number }) {
  if (count <= 0) return null;
  return (
    <span
      className="absolute -right-2.5 -top-1 min-w-[1.1rem] rounded-full bg-primary px-1 text-center text-[10px] font-semibold leading-[1.1rem] text-primary-foreground"
      data-testid="phone-tab-badge"
    >
      {count > 99 ? "99+" : count}
    </span>
  );
}

export function PhoneTabBar({
  active,
  badges,
  onSelect,
}: {
  active: PhoneTab;
  badges: Partial<Record<PhoneTab, number>>;
  onSelect: (tab: PhoneTab) => void;
}) {
  return (
    <nav
      aria-label="Primary"
      className="phone-tabbar fixed inset-x-0 bottom-0 z-[120] flex border-t border-sidebar-border bg-sidebar/95 text-sidebar-foreground backdrop-blur"
      data-testid="phone-tabbar"
    >
      {TABS.map(({ id, label, Icon }) => {
        const isActive = id === active;
        return (
          <button
            aria-current={isActive ? "page" : undefined}
            className={cn(
              "flex min-w-0 flex-1 flex-col items-center gap-0.5 pb-1 pt-1.5 text-[10px] font-medium",
              isActive ? "text-primary" : "text-muted-foreground",
            )}
            data-testid={`phone-tab-${id}`}
            key={id}
            onClick={() => onSelect(id)}
            type="button"
          >
            <span className="relative">
              <Icon className="h-6 w-6" />
              <Badge count={badges[id] ?? 0} />
            </span>
            <span className="truncate">{label}</span>
          </button>
        );
      })}
    </nav>
  );
}

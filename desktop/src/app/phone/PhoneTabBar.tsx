// Phone-only bottom tab bar (rendered only when usePhoneLayout() is true). iPad/desktop never mount it.
import { Inbox, MessagesSquare, Search, SquareTerminal } from "lucide-react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { useSidebarMenuCounts } from "@/features/sidebar/lib/useSidebarMenuCounts";
import type * as React from "react";

import { cn } from "@/shared/lib/cn";
import type { PhoneTab } from "./phoneTabs";

// Me (avatar) first, then Inbox, Channels, Buzz Term, Search.
const TABS: { id: PhoneTab; label: string; Icon?: React.ComponentType<{ className?: string }> }[] = [
  { id: "me", label: "Me" },
  { id: "inbox", label: "Inbox", Icon: Inbox },
  { id: "channels", label: "Channels", Icon: MessagesSquare },
  { id: "term", label: "Buzz Term", Icon: SquareTerminal },
  { id: "search", label: "Search", Icon: Search },
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
  me,
  onSelect,
}: {
  active: PhoneTab;
  me: { avatarUrl?: string | null; displayName: string; pubkey?: string };
  badges: Partial<Record<PhoneTab, number>>;
  onSelect: (tab: PhoneTab) => void;
}) {
  // Inbox unread follows the Appearance "Show counts on menu items" Inbox toggle (same source as the sidebar).
  const { preferences, counts } = useSidebarMenuCounts();
  const tabBadges: Partial<Record<PhoneTab, number>> = {
    ...badges,
    inbox: preferences.inbox ? (counts.inbox ?? 0) : 0,
  };
  return (
    <nav
      aria-label="Primary"
      className="phone-tabbar z-[120] flex text-white"
      data-testid="phone-tabbar"
    >
      {TABS.map(({ id, label, Icon }) => {
        const isActive = id === active;
        return (
          <button
            aria-label={label}
            aria-current={isActive ? "page" : undefined}
            className={cn(
              "phone-tab flex min-w-0 flex-1 flex-col items-center justify-center gap-0.5 rounded-full text-[10px] font-medium",
              isActive ? "text-white" : "text-white/60",
            )}
            data-testid={`phone-tab-${id}`}
            key={id}
            onClick={() => onSelect(id)}
            type="button"
          >
            {Icon ? (
              <>
                <span className="relative">
                  <Icon className="h-[22px] w-[22px]" />
                  <Badge count={tabBadges[id] ?? 0} />
                </span>
                <span className="truncate">{label}</span>
              </>
            ) : (
              <span className="relative" data-testid="phone-tab-me-avatar">
                <ProfileAvatar
                  avatarUrl={me.avatarUrl ?? null}
                  className="phone-tab-avatar text-[11px]"
                  label={me.displayName}
                  pubkey={me.pubkey}
                />
                <span
                  aria-label="Online"
                  className="absolute -bottom-0.5 -right-0.5 h-3 w-3 rounded-full border-2 border-zinc-900 bg-emerald-500"
                  data-testid="phone-tab-me-online"
                />
              </span>
            )}
          </button>
        );
      })}
    </nav>
  );
}

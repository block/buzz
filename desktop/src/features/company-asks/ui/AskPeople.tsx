import { AppTopChromePortal } from "@/app/AppTopChromePortal";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { cn } from "@/shared/lib/cn";

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] as const;
const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
] as const;

const AVATAR_SIZE_CLASS = {
  24: "size-6",
  28: "size-7",
  30: "size-[1.875rem]",
} as const;

/**
 * Round for people, the squircle ant picture for agents (owner decision A2:
 * Colony 3.0 agents are not hexagons). `size` is the board's px size.
 */
export function AskAvatar({
  avatarUrl,
  pubkey,
  isAgent = false,
  label,
  size = 28,
}: {
  avatarUrl?: string | null;
  pubkey?: string | null;
  isAgent?: boolean;
  label: string;
  size?: 24 | 28 | 30;
}) {
  return (
    <span aria-hidden="true" className="contents">
      <ProfileAvatar
        avatarUrl={avatarUrl ?? null}
        className={cn("ask3-av", AVATAR_SIZE_CLASS[size])}
        label={label}
        pubkey={pubkey}
        shape={isAgent ? "squircle" : "circle"}
      />
    </span>
  );
}

/** "Today, 13:52" for the current day, "Wed 21 Oct" for any other day. */
export function formatThreadWhen(createdAtSeconds: number, now = new Date()) {
  const created = new Date(createdAtSeconds * 1_000);
  const sameDay =
    created.getFullYear() === now.getFullYear() &&
    created.getMonth() === now.getMonth() &&
    created.getDate() === now.getDate();
  if (sameDay) {
    const hours = String(created.getHours()).padStart(2, "0");
    const minutes = String(created.getMinutes()).padStart(2, "0");
    return `Today, ${hours}:${minutes}`;
  }
  return `${WEEKDAYS[created.getDay()]} ${created.getDate()} ${MONTHS[created.getMonth()]}`;
}

/** First word of a display name, for example "Started by Alex". */
export function firstName(name: string) {
  return name.trim().split(/\s+/)[0] ?? name;
}

/**
 * The shell top bar trail for the ask screens ("Company / Raise an ask"). The
 * route has no default breadcrumb, so the screen supplies its own.
 */
export function AskTopBarTrail({ trail }: { trail: readonly string[] }) {
  return (
    <AppTopChromePortal>
      <nav
        aria-label="Breadcrumb"
        className="ask3-trail"
        data-testid="top-bar-breadcrumb"
      >
        {trail.map((label, index) => {
          const last = index === trail.length - 1;
          return (
            <span
              aria-current={last ? "page" : undefined}
              className="contents"
              key={label}
            >
              {index > 0 ? <span aria-hidden="true">/</span> : null}
              <span data-current={last ? "true" : undefined}>{label}</span>
            </span>
          );
        })}
      </nav>
    </AppTopChromePortal>
  );
}

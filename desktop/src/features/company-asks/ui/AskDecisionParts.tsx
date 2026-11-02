import type * as React from "react";
import { Link } from "@tanstack/react-router";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  CheckIcon,
  PrivateChannelIcon,
  SearchIcon,
} from "@/shared/ui/colonyIcons";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { OutcomeChip, OutcomeIcon } from "./askOutcomeModel";
import "./askDecisions.css";

function Glyph({
  children,
  size,
  strokeWidth,
}: {
  children: React.ReactNode;
  size: number;
  strokeWidth: number;
}) {
  const length = `${size / 16}rem`;
  return (
    <svg
      aria-hidden="true"
      fill="none"
      height={length}
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={strokeWidth}
      viewBox="0 0 24 24"
      width={length}
    >
      {children}
    </svg>
  );
}

export function ShieldGlyph() {
  return (
    <Glyph size={16} strokeWidth={2.2}>
      <path d="M12 3.5 19 6v6c0 4.5-3 7.5-7 8.5-4-1-7-4-7-8.5V6z" />
    </Glyph>
  );
}

export function LockGlyph() {
  return <PrivateChannelIcon size={17} strokeWidth={2.2} />;
}

export function SearchGlyph() {
  return <SearchIcon size={19} strokeWidth={2.2} />;
}

export function BellGlyph() {
  return (
    <Glyph size={19} strokeWidth={2}>
      <path d="M6 9.5a6 6 0 0 1 12 0c0 6 2.5 7.5 2.5 7.5h-17S6 15.5 6 9.5z" />
      <path d="M10 20.5a2.2 2.2 0 0 0 4 0" />
    </Glyph>
  );
}

export function ChecklistTick() {
  return <CheckIcon size={15} strokeWidth={3} />;
}

function ChipGlyph({ icon }: { icon: OutcomeIcon }) {
  switch (icon) {
    case "check":
      return <CheckIcon size={12} strokeWidth={3.2} />;
    case "x":
      return (
        <Glyph size={12} strokeWidth={3}>
          <path d="M6 6l12 12M18 6 6 18" />
        </Glyph>
      );
    case "pencil":
      return (
        <Glyph size={12} strokeWidth={2.6}>
          <path d="M4 20h4L19 9l-4-4L4 16z" />
        </Glyph>
      );
    case "alert":
      return (
        <Glyph size={12} strokeWidth={2.8}>
          <circle cx="12" cy="12" r="8.5" />
          <path d="M12 8v4.5M12 16v.2" />
        </Glyph>
      );
    default:
      return null;
  }
}

/** The pill on an outcome card: its colour and glyph follow the outcome. */
export function OutcomeChipView({
  chip,
  testId,
}: {
  chip: OutcomeChip;
  testId?: string;
}) {
  return (
    <span
      className="colony-outcome-chip"
      data-testid={testId}
      data-tone={chip.tone}
    >
      <ChipGlyph icon={chip.icon} />
      {chip.label}
    </span>
  );
}

/** A status pill on the decision pages (Open, Needs you, Overdue). */
export function DecisionChip({
  children,
  tone = "idle",
  testId,
}: {
  children: React.ReactNode;
  tone?: "idle" | "need" | "bad" | "ok" | "info";
  testId?: string;
}) {
  const style =
    tone === "need"
      ? {
          background: "hsl(var(--colony-need-bg))",
          color: "hsl(var(--colony-need))",
        }
      : tone === "bad"
        ? {
            background: "hsl(var(--colony-danger-bg))",
            color: "hsl(var(--colony-danger))",
          }
        : tone === "ok"
          ? {
              background: "hsl(var(--colony-success-bg))",
              color: "hsl(var(--colony-success))",
            }
          : tone === "info"
            ? {
                background: "hsl(var(--colony-info-bg))",
                color: "hsl(var(--colony-info))",
              }
            : undefined;
  return (
    <span className="colony-decision-chip" data-testid={testId} style={style}>
      {children}
    </span>
  );
}

/** A person (round) or an AI employee (their own squircle picture). */
export function DecisionAvatar({
  pubkey,
  name,
  profiles,
  isAgent,
  size,
}: {
  pubkey?: string | null;
  name: string;
  profiles?: UserProfileLookup;
  isAgent?: boolean;
  size: 22 | 24 | 28 | 42;
}) {
  const profile = pubkey ? profiles?.[normalizePubkey(pubkey)] : undefined;
  const agent = isAgent ?? profile?.isAgent === true;
  return (
    <ProfileAvatar
      avatarUrl={profile?.avatarUrl ?? null}
      className={cn("colony-decision-avatar", `colony-decision-avatar-${size}`)}
      label={name}
      pubkey={pubkey}
      shape={agent ? "squircle" : "circle"}
    />
  );
}

/** "You can view this ask, but cannot respond." */
export function CannotRespond({
  message,
  alert,
}: {
  message: string;
  alert: boolean;
}) {
  return (
    <div
      className="colony-outcome-denied"
      data-testid="ask-cannot-resolve"
      role={alert ? "alert" : "status"}
    >
      <LockGlyph />
      <span>
        <strong>You can view this ask, but cannot respond.</strong>
        <span>{message}</span>
      </span>
    </div>
  );
}

export type LinkedWorkView = {
  workItemId: string;
  clientId: string;
  title: string;
  status: string;
  statusLabel: string;
  /** "Client workspace · team member": the channel only when not the ask's own. */
  meta: string;
};

export function linkedWorkTone(
  status: string,
  funding: boolean,
): "accent" | "need" | "bad" | "ok" | "idle" {
  if (status === "review") return funding ? "need" : "accent";
  if (status === "blocked") return "bad";
  if (status === "complete") return "ok";
  if (status === "paused" || status === "archived") return "idle";
  return "accent";
}

export const WORK_STATUS_LABELS: Record<string, string> = {
  active: "In progress",
  review: "Needs review",
  blocked: "Blocked",
  paused: "Paused",
  complete: "Complete",
  archived: "Archived",
};

/** The right-hand "Decision context" panel of the decision pages. */
export function DecisionContext({
  addresseeName,
  addresseePubkey,
  profiles,
  deadline,
  overdue,
  linkedWork,
  funding,
}: {
  addresseeName: string | null;
  addresseePubkey: string | null;
  profiles?: UserProfileLookup;
  deadline: string | null;
  overdue: boolean;
  linkedWork: LinkedWorkView | null;
  funding: boolean;
}) {
  return (
    <aside aria-label="Decision context" className="colony-decision-aside">
      <h2>Decision context</h2>
      <dl>
        <div data-gap="6">
          <dt>Addressed to</dt>
          <dd data-person={addresseeName ? "true" : undefined}>
            {addresseeName ? (
              <>
                <DecisionAvatar
                  name={addresseeName}
                  profiles={profiles}
                  pubkey={addresseePubkey}
                  size={28}
                />
                {addresseeName}
              </>
            ) : (
              "Owner or administrator"
            )}
          </dd>
        </div>
        <div>
          <dt>Can decide</dt>
          <dd>Owner or administrator</dd>
        </div>
        <div>
          <dt>Deadline</dt>
          <dd>
            {deadline ?? "No deadline"}
            {deadline && overdue ? (
              <>
                {" · "}
                <span className="colony-decision-overdue">overdue</span>
              </>
            ) : null}
          </dd>
        </div>
        {linkedWork ? (
          <div data-gap="8">
            <dt>Linked work</dt>
            <dd className="colony-decision-linked">
              <strong>{linkedWork.title}</strong>
              <span className="colony-decision-linked-meta">
                <span
                  className="colony-decision-linked-chip"
                  data-tone={linkedWorkTone(linkedWork.status, funding)}
                >
                  {linkedWork.statusLabel}
                </span>
                {linkedWork.meta ? <span>{linkedWork.meta}</span> : null}
              </span>
              <Link
                aria-label={`Open work item ${linkedWork.title}`}
                className="colony-decision-open-work"
                params={{ workId: linkedWork.workItemId }}
                search={{ client: linkedWork.clientId }}
                to="/work/$workId"
              >
                Open work
              </Link>
            </dd>
          </div>
        ) : null}
      </dl>
    </aside>
  );
}

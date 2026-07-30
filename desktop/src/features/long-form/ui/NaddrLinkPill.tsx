import * as React from "react";
import { BookOpen } from "lucide-react";

import { cn } from "@/shared/lib/cn";

import { parseNaddrUri } from "../lib/nostrAddress";
import { LongFormNoteDialog } from "./LongFormNoteDialog";

export function renderInteractiveNaddrLink(
  href: string,
  children: React.ReactNode,
) {
  return (
    <NaddrLinkPill href={href} interactive>
      {children}
    </NaddrLinkPill>
  );
}

export function NaddrLinkPill({
  children,
  className,
  href,
  interactive,
}: {
  children?: React.ReactNode;
  className?: string;
  href: string;
  interactive: boolean;
}) {
  const [hasOpened, setHasOpened] = React.useState(false);
  const [open, setOpen] = React.useState(false);
  const address = parseNaddrUri(href);
  const label = children ?? href;

  if (!address || !interactive) {
    return (
      <span className={className} data-naddr-link="">
        {label}
      </span>
    );
  }

  return (
    <>
      <button
        className={cn(
          "inline-flex max-w-full cursor-pointer items-center gap-1 rounded-md bg-primary/10 px-1.5 py-0.5 font-medium text-primary transition-colors hover:bg-primary/15 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
          className,
        )}
        data-naddr-link=""
        onClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          setHasOpened(true);
          setOpen(true);
        }}
        type="button"
      >
        <BookOpen aria-hidden="true" className="h-3.5 w-3.5 shrink-0" />
        <span className="min-w-0 truncate">{label}</span>
      </button>
      {hasOpened ? (
        <LongFormNoteDialog
          address={address}
          onOpenChange={setOpen}
          open={open}
        />
      ) : null}
    </>
  );
}

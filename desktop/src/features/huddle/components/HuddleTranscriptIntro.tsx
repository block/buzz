import { MessageCircle } from "lucide-react";

/** Orientation content shown at the top of the companion huddle transcript. */
export function HuddleTranscriptIntro() {
  return (
    <div
      className="mx-1 flex items-start gap-2.5 rounded-2xl border border-border/60 bg-muted/30 px-2 py-2.5 text-left"
      data-testid="huddle-transcript-intro"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <MessageCircle aria-hidden className="h-4 w-4" />
      </span>
      <div className="min-w-0">
        <p className="text-base font-semibold text-foreground">Huddle chat</p>
        <p className="mt-0.5 max-w-xl text-sm leading-5 text-muted-foreground">
          Chat with huddle participants and agents. The transcript appears here
          too.
        </p>
      </div>
    </div>
  );
}

import { MessageCircle } from "lucide-react";

import { useTranslation } from "@/i18n";

/** Orientation content shown at the top of the companion huddle transcript. */
export function HuddleTranscriptIntro() {
  const { t } = useTranslation();
  return (
    <div
      className="mx-1 flex items-start gap-2.5 rounded-2xl border border-border/60 bg-muted/30 px-2 py-2.5 text-left"
      data-testid="huddle-transcript-intro"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <MessageCircle aria-hidden className="h-4 w-4" />
      </span>
      <div className="min-w-0">
        <p className="text-base font-semibold text-foreground">
          {t("huddle.transcript-intro.title")}
        </p>
        <p className="mt-0.5 max-w-xl text-sm leading-5 text-muted-foreground">
          {t("huddle.transcript-intro.body")}
        </p>
      </div>
    </div>
  );
}

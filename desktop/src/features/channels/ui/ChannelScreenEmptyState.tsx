// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
export function ChannelScreenEmptyState() {
  return (
    <div className="flex min-h-0 flex-1 items-center justify-center px-6 py-8">
      <p className="text-sm text-muted-foreground">
        {uiT("Select a channel to view messages.")}
      </p>
    </div>
  );
}

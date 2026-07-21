import type { DraftViewItem } from "@/features/messages/ui/DraftsPanel";
import { DraftDetailPane } from "@/features/messages/ui/DraftDetailPane";
import type { Reminder } from "@/features/reminders/lib/reminderTypes";
import { ReminderDetailPane } from "@/features/reminders/ui/RemindersPanel";

type HomePersonalActivityDetailProps = {
  currentPubkey?: string;
  draftItem: DraftViewItem | null;
  mode: "drafts" | "reminders";
  onBack?: () => void;
  onDeleteDraft: (draftKey: string) => void;
  reminder: Reminder | null;
};

export function HomePersonalActivityDetail({
  currentPubkey,
  draftItem,
  mode,
  onBack,
  onDeleteDraft,
  reminder,
}: HomePersonalActivityDetailProps) {
  if (mode === "drafts") {
    return (
      <DraftDetailPane
        item={draftItem}
        key={draftItem?.entry.key ?? "empty"}
        onBack={onBack}
        onDelete={onDeleteDraft}
      />
    );
  }

  if (mode === "reminders") {
    return (
      <ReminderDetailPane
        onBack={onBack}
        pubkey={currentPubkey ?? ""}
        reminder={reminder}
      />
    );
  }

  return null;
}

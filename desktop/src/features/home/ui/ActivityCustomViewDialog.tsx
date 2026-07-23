import * as React from "react";

import type { ActivityCustomView } from "@/features/home/lib/activityViewPreferences";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

const SOURCE_OPTIONS: Array<{
  key: keyof ActivityCustomView;
  label: string;
}> = [
  { key: "dms", label: "Direct messages" },
  { key: "mentions", label: "Mentions" },
  { key: "threads", label: "Threads" },
  { key: "agentReplies", label: "Agent replies" },
  { key: "needsAction", label: "Needs action" },
  { key: "dueReminders", label: "Due reminders" },
  { key: "drafts", label: "Drafts" },
];

type ActivityCustomViewDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (value: ActivityCustomView) => void;
  value: ActivityCustomView;
};

export function ActivityCustomViewDialog({
  open,
  onOpenChange,
  onSave,
  value,
}: ActivityCustomViewDialogProps) {
  const [draft, setDraft] = React.useState(value);

  React.useEffect(() => {
    if (open) setDraft(value);
  }, [open, value]);

  const hasSelectedSource = SOURCE_OPTIONS.some(({ key }) => draft[key]);
  const update = (key: keyof ActivityCustomView, checked: boolean) => {
    setDraft((current) => ({ ...current, [key]: checked }));
  };

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="sm:max-w-[420px]">
        <DialogHeader>
          <DialogTitle>Custom view</DialogTitle>
          <DialogDescription>
            Choose what appears in your custom Activity view for this community.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-1">
          {SOURCE_OPTIONS.map((option) => {
            const id = `activity-custom-${option.key}`;
            return (
              <label
                className="flex min-h-10 cursor-pointer items-center gap-3 rounded-md px-2 transition-colors hover:bg-muted/50"
                htmlFor={id}
                key={option.key}
              >
                <Checkbox
                  checked={draft[option.key]}
                  id={id}
                  onCheckedChange={(checked) =>
                    update(option.key, checked === true)
                  }
                />
                <span className="text-sm font-medium">{option.label}</span>
              </label>
            );
          })}
        </div>

        <DialogFooter>
          <Button onClick={() => onOpenChange(false)} variant="ghost">
            Cancel
          </Button>
          <Button
            disabled={!hasSelectedSource}
            onClick={() => {
              onSave(draft);
              onOpenChange(false);
            }}
          >
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

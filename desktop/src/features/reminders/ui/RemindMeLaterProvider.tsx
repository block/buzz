import * as React from "react";

import type { ReminderTarget } from "@/features/reminders/lib/reminderTypes";
import { RemindMeLaterDialog } from "./RemindMeLaterDialog";

type RemindMeLaterContextValue = {
  openReminder: (target: ReminderTarget) => void;
};

const RemindMeLaterContext = React.createContext<RemindMeLaterContextValue>({
  openReminder: () => {},
});

export function useRemindLater() {
  return React.useContext(RemindMeLaterContext);
}

export function RemindMeLaterProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const [open, setOpen] = React.useState(false);
  const [target, setTarget] = React.useState<ReminderTarget | null>(null);

  const openReminder = React.useCallback((t: ReminderTarget) => {
    setTarget(t);
    setOpen(true);
  }, []);

  const contextValue = React.useMemo(() => ({ openReminder }), [openReminder]);

  return (
    <RemindMeLaterContext.Provider value={contextValue}>
      {children}
      <RemindMeLaterDialog open={open} onOpenChange={setOpen} target={target} />
    </RemindMeLaterContext.Provider>
  );
}

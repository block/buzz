import { createFileRoute, redirect } from "@tanstack/react-router";

// Reminders is now a view mode inside the inbox (`/?view=reminders`), not a
// standalone screen. This redirect preserves existing history entries and
// bookmarks pointing at `/reminders` — they land in the inbox Reminders view
// instead of dead-ending.
export const Route = createFileRoute("/reminders")({
  beforeLoad: () => {
    throw redirect({ to: "/", search: { view: "reminders" } });
  },
});

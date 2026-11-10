const REMINDER_NOTIFICATION_POLL_INTERVAL_MS = 30_000;

/** Keep due-reminder detection alive while Buzz is hidden or unfocused. */
export function startReminderNotificationPoll(check: () => void): () => void {
  check();
  const interval = window.setInterval(
    check,
    REMINDER_NOTIFICATION_POLL_INTERVAL_MS,
  );
  return () => window.clearInterval(interval);
}

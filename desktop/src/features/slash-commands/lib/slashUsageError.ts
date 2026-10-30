/**
 * Thrown by a command's `run` when its arguments are wrong. The executor
 * shows the command's usage line, keeps the draft, and publishes nothing.
 */
export class SlashUsageError extends Error {
  readonly detail: string | null;

  constructor(detail?: string) {
    super(detail ?? "Invalid command arguments");
    this.name = "SlashUsageError";
    this.detail = detail ?? null;
  }
}

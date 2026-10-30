/**
 * Thrown by a command's `run` when the user dismisses a confirmation the
 * command opened. The executor reports no toast and keeps the draft: the
 * outcome is neither a success nor a failure.
 */
export class SlashCommandCancelled extends Error {
  constructor() {
    super("Command cancelled");
    this.name = "SlashCommandCancelled";
  }
}

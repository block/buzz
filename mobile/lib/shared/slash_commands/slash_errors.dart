/// Thrown by a command when its arguments are wrong. The executor shows the
/// command's usage line, preceded by [reason] when given, and publishes
/// nothing.
class SlashUsageError implements Exception {
  final String? reason;

  const SlashUsageError([this.reason]);

  @override
  String toString() => reason ?? 'SlashUsageError';
}

/// Thrown by a command for an expected failure whose [message] is fit to show
/// the user as is.
class SlashCommandFailure implements Exception {
  final String message;

  const SlashCommandFailure(this.message);

  @override
  String toString() => message;
}

/// Thrown by a command when the user cancels one of its confirmations. It is
/// neither a success nor an error: the executor keeps the draft and shows
/// nothing.
class SlashCommandCancelled implements Exception {
  const SlashCommandCancelled();

  @override
  String toString() => 'SlashCommandCancelled';
}

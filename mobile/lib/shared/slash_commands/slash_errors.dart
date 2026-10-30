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

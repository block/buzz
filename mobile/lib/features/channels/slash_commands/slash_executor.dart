import '../../../shared/slash_commands/slash_errors.dart';
import 'slash_command.dart';

/// Runs [command] and reports failure to the user. Returns true only when the
/// command succeeded, which is the only case where the composer clears the
/// draft. Nothing is retried.
Future<bool> executeSlashCommand(
  SlashCommand command,
  SlashCommandContext ctx,
) async {
  if (!command.availableIn(isDm: ctx.isDm)) {
    ctx.showMessage('/${command.name} only works in channels');
    return false;
  }
  try {
    await command.run(ctx);
    return true;
  } on SlashCommandCancelled {
    return false;
  } on SlashUsageError catch (error) {
    final reason = error.reason;
    final usage = 'Usage: ${command.usage}';
    ctx.showMessage(reason == null ? usage : '$reason\n$usage');
  } on SlashCommandFailure catch (error) {
    ctx.showMessage(error.message);
  } on StateError {
    ctx.showMessage('/${command.name} not run: the community changed');
  } catch (error) {
    ctx.showMessage(error.toString().replaceFirst('Exception: ', ''));
  }
  return false;
}

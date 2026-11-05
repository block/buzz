import '../../../shared/slash_commands/slash_recognizer.dart';
import 'groups/channel_admin_commands.dart';
import 'groups/membership_commands.dart';
import 'groups/navigation_commands.dart';
import 'groups/presence_commands.dart';
import 'groups/remind_commands.dart';
import 'groups/text_commands.dart';
import 'slash_command.dart';

/// A fixed set of commands with case-insensitive lookup by name or alias.
class SlashRegistry {
  final List<SlashCommand> commands;
  final Map<String, SlashCommand> _byName;

  SlashRegistry(Iterable<SlashCommand> commands)
    : commands = List.unmodifiable(commands),
      _byName = _index(commands);

  static Map<String, SlashCommand> _index(Iterable<SlashCommand> commands) {
    final byName = <String, SlashCommand>{};
    for (final command in commands) {
      for (final name in [command.name, ...command.aliases]) {
        final previous = byName[name];
        if (previous != null) {
          throw StateError(
            '/$name is claimed by both /${previous.name} and /${command.name}',
          );
        }
        byName[name] = command;
      }
    }
    return byName;
  }

  /// The command [name] (already lowercase) names, aliases included.
  SlashCommand? lookup(String name) => _byName[name];

  /// Spec "Recognition".
  SlashInvocation<SlashCommand>? recognize(String draft) =>
      recognizeSlashDraft(draft, lookup);

  /// Spec "Autocomplete": commands for this composer and context whose name
  /// starts with [query], alphabetical. Aliases are never matched or listed.
  List<SlashCommand> suggest(String query, SlashVisibilityContext context) {
    final prefix = query.toLowerCase();
    return [
      for (final command in commands)
        if (command.name.startsWith(prefix) &&
            command.availableIn(isDm: context.isDm) &&
            (command.isVisible?.call(context) ?? true))
          command,
    ]..sort((a, b) => a.name.compareTo(b.name));
  }
}

/// Every command, composed from one file per group so groups never conflict.
final slashRegistry = SlashRegistry([
  ...presenceSlashCommands,
  ...membershipSlashCommands,
  ...channelAdminSlashCommands,
  ...navigationSlashCommands,
  ...remindSlashCommands,
  ...textSlashCommands,
]);

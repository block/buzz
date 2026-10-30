import 'package:characters/characters.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../../shared/custom_emoji/custom_emoji.dart';
import '../../../../shared/custom_emoji/custom_emoji_provider.dart';
import '../../../../shared/emoji/emoji_data.dart';
import '../../../../shared/emoji/emoji_data_provider.dart';
import '../../../../shared/relay/relay.dart';
import '../../../../shared/slash_commands/slash_errors.dart';
import '../../../../shared/slash_commands/slash_tokenizer.dart';
import '../../../profile/profile_provider.dart';
import '../../../profile/set_status_sheet.dart';
import '../../../profile/user_status_provider.dart';
import '../slash_command.dart';

final _emojiGrapheme = RegExp(
  // ignore: valid_regexps, the analyzer does not know Unicode property escapes
  r'\p{Emoji_Presentation}|\p{Extended_Pictographic}\uFE0F|[0-9#*]\uFE0F?\u20E3',
  unicode: true,
);

final _shortcodeToken = RegExp(r'^:[^:\s]+:$');

/// A status split from `/status` arguments. [emoji] is a glyph, or a custom
/// emoji's `:shortcode:`, as the set-status sheet stores it.
typedef StatusParts = ({String emoji, String text});

/// Whether `/status` arguments need the standard emoji dataset to be split.
bool statusArgsNeedEmojiDataset(String args) {
  final tokens = tokenizeSlashArgs(args);
  return tokens.isNotEmpty && _shortcodeToken.hasMatch(tokens.first.text);
}

/// Splits `[emoji] <text>`. The first argument is the emoji only when it is a
/// single emoji grapheme or a `:shortcode:` found in [customEmoji] or
/// [dataset]; otherwise all of [args] is the text.
StatusParts splitStatusArgs(
  String args, {
  required List<CustomEmoji> customEmoji,
  required EmojiDataset dataset,
}) {
  final tokens = tokenizeSlashArgs(args);
  if (tokens.isEmpty) return (emoji: '', text: '');
  final first = tokens.first.text;
  final emoji = _shortcodeToken.hasMatch(first)
      ? _resolveShortcode(first, customEmoji, dataset)
      : _isEmojiGrapheme(first)
      ? first
      : null;
  if (emoji == null) return (emoji: '', text: args.trim());
  return (emoji: emoji, text: slashArgsFrom(args, 1));
}

bool _isEmojiGrapheme(String token) =>
    token.characters.length == 1 && _emojiGrapheme.hasMatch(token);

String? _resolveShortcode(
  String token,
  List<CustomEmoji> customEmoji,
  EmojiDataset dataset,
) {
  final custom = normalizeShortcode(token);
  if (custom != null && customEmoji.any((e) => e.shortcode == custom)) {
    return ':$custom:';
  }
  final id = token.substring(1, token.length - 1).toLowerCase();
  for (final entry in dataset.all) {
    if (entry.skinIndex == 0 && entry.id == id) return entry.native;
  }
  return null;
}

Future<void> _setPresence(
  SlashCommandContext ctx,
  String status,
  String toast,
) async {
  if (ctx.args.isNotEmpty) throw const SlashUsageError();
  final notifier = ctx.ref.read(presenceProvider.notifier);
  final built = ctx.ref.read(presenceProvider.future);
  await built;
  await notifier.setPresence(status);
  ctx.showMessage(toast);
}

Future<void> _runStatus(SlashCommandContext ctx) async {
  final args = ctx.args;
  final ref = ctx.ref;
  final notifier = ref.read(userStatusProvider.notifier);

  if (args.isEmpty) {
    showSetStatusSheet(
      ctx.context,
      currentStatus: ref.read(userStatusProvider).asData?.value,
    );
    return;
  }

  final nsec = ref.read(relayConfigProvider).nsec;
  if (nsec == null || nsec.isEmpty) {
    throw const SlashCommandFailure('Sign in to set a status');
  }

  if (args.toLowerCase() == 'clear') {
    await notifier.clearStatus();
    ctx.showMessage('Status cleared');
    return;
  }

  final customEmoji = ref.read(customEmojiListProvider);
  final dataset = statusArgsNeedEmojiDataset(args)
      ? await ref.read(emojiDatasetProvider.future)
      : EmojiDataset.empty;
  final parts = splitStatusArgs(
    args,
    customEmoji: customEmoji,
    dataset: dataset,
  );
  await notifier.setStatus(parts.text, parts.emoji);
  ctx.showMessage('Status set');
}

final List<SlashCommand> presenceSlashCommands = [
  SlashCommand(
    name: 'away',
    description: 'Set yourself as away',
    usage: '/away',
    run: (ctx) => _setPresence(ctx, 'away', "You're away"),
  ),
  SlashCommand(
    name: 'active',
    description: 'Set yourself as active',
    usage: '/active',
    run: (ctx) => _setPresence(ctx, 'online', "You're active"),
  ),
  SlashCommand(
    name: 'status',
    argumentHint: '[emoji] <text> | clear',
    description: 'Set or clear your status',
    usage: '/status [emoji] <text> | /status clear',
    run: _runStatus,
  ),
];

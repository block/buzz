import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/slash_commands/slash_tokenizer.dart';
import '../channel.dart';

/// Where a command can run. A thread acts on its channel.
enum SlashScope {
  /// A channel, DM, or thread.
  any,

  /// A non-DM channel, or a thread in one.
  channel,
}

/// A person the draft mentioned, resolved by the composer's mention rules.
typedef SlashMention = ({String pubkey, String label});

/// Posts [content] through the composer's normal send path. Custom emoji tags
/// are added; [mentionPubkeys] defaults to none.
typedef SlashSendMessage =
    Future<void> Function(String content, {List<String> mentionPubkeys});

/// What a visibility predicate may inspect. Predicates run while the composer
/// builds, so they may `ref.watch`, and must be synchronous and cheap.
class SlashVisibilityContext {
  final WidgetRef ref;
  final String channelId;
  final Channel? channel;
  final bool isDm;
  final bool inThread;

  const SlashVisibilityContext({
    required this.ref,
    required this.channelId,
    required this.channel,
    required this.isDm,
    required this.inThread,
  });
}

/// Everything a command handler receives. Read what you need from [ref]
/// before the first `await`: the composer can unmount while a command runs.
class SlashCommandContext {
  final WidgetRef ref;

  /// The composer's context, for `Navigator.of(context)` and the like. Check
  /// `context.mounted` after an `await`.
  final BuildContext context;

  final String channelId;
  final Channel? channel;
  final bool isDm;

  /// The thread's head event id; null in the main channel composer.
  final String? threadHeadId;
  final String? rootId;

  /// The text after the command token, trimmed.
  final String args;
  final List<SlashToken> tokens;

  /// Mentions in the draft, resolved by the composer's exact-key rules.
  final List<SlashMention> mentions;

  /// `#channel` references in [args] that match a channel the user can see,
  /// in order of appearance.
  final List<Channel> channels;

  /// `#names` in [args] that match no visible channel.
  final List<String> unresolvedChannelNames;

  final SlashSendMessage sendMessage;

  /// Shows [message] in the app's snackbar. Safe after an `await`.
  final void Function(String message) showMessage;

  const SlashCommandContext({
    required this.ref,
    required this.context,
    required this.channelId,
    required this.channel,
    required this.isDm,
    required this.threadHeadId,
    required this.rootId,
    required this.args,
    required this.tokens,
    required this.mentions,
    required this.channels,
    required this.unresolvedChannelNames,
    required this.sendMessage,
    required this.showMessage,
  });

  bool get inThread => threadHeadId != null;
  List<String> get mentionPubkeys => [for (final m in mentions) m.pubkey];
  List<String> get channelIds => [for (final c in channels) c.id];
}

typedef SlashRun = Future<void> Function(SlashCommandContext ctx);

/// A composer command. Throw `SlashUsageError` for bad arguments and
/// `SlashCommandFailure` for an expected failure; any other error is shown
/// too. Either way nothing clears the draft.
@immutable
class SlashCommand {
  /// Lowercase, without the slash; letters, digits, `_` and `-` only.
  final String name;

  /// Lowercase names that run this command but are never suggested.
  final List<String> aliases;

  /// Shown after the name in suggestions, for example `[text]`.
  final String argumentHint;

  /// One line, shown in suggestions.
  final String description;

  /// Shown after a usage error, for example `/rename <name>`.
  final String usage;

  final SlashScope scope;

  /// Hides the command from suggestions when it returns false. A hidden
  /// command still runs when typed in full; the relay stays the authority.
  final bool Function(SlashVisibilityContext)? isVisible;

  final SlashRun run;

  const SlashCommand({
    required this.name,
    this.aliases = const [],
    this.argumentHint = '',
    required this.description,
    required this.usage,
    this.scope = SlashScope.any,
    this.isVisible,
    required this.run,
  });

  bool availableIn({required bool isDm}) => scope == SlashScope.any || !isDm;
}

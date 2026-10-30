part of '../compose_bar.dart';

typedef _SlashComposer = ({
  List<SlashCommand> suggestions,
  void Function(SlashCommand) select,
  Future<bool> Function({
    required SlashInvocation<SlashCommand> invocation,
    required List<MentionCandidate> mentions,
    required ComposeBarOnSend send,
    required ScaffoldMessengerState? messenger,
  })
  run,
});

_SlashComposer _useSlashCommands({
  required WidgetRef ref,
  required bool enabled,
  required String channelId,
  required Channel? channel,
  required String? threadHeadId,
  required String? rootId,
  required List<Channel> channels,
  required List<CustomEmoji> customEmoji,
  required _MarkdownEditingController controller,
  required FocusNode focusNode,
}) {
  final context = useContext();
  final isDm = channel?.isDm ?? false;
  SlashQuery? currentQuery() => enabled && controller.selection.isCollapsed
      ? findSlashQuery(controller.text, controller.selection.baseOffset)
      : null;
  final query = useListenableSelector(controller, currentQuery);
  final suggestions = query == null
      ? const <SlashCommand>[]
      : slashRegistry.suggest(
          query.query,
          SlashVisibilityContext(
            ref: ref,
            channelId: channelId,
            channel: channel,
            isDm: isDm,
            inThread: threadHeadId != null,
          ),
        );

  void select(SlashCommand command) {
    final open = currentQuery();
    if (open == null) return;
    final text = controller.text;
    final end = open.end < text.length && text[open.end] == ' '
        ? open.end + 1
        : open.end;
    final replacement = '/${command.name} ';
    controller.value = TextEditingValue(
      text: text.replaceRange(open.start, end, replacement),
      selection: TextSelection.collapsed(
        offset: open.start + replacement.length,
      ),
    );
    focusNode.requestFocus();
  }

  Future<bool> run({
    required SlashInvocation<SlashCommand> invocation,
    required List<MentionCandidate> mentions,
    required ComposeBarOnSend send,
    required ScaffoldMessengerState? messenger,
  }) {
    final refs = resolveSlashChannelRefs(invocation.args, channels);
    return executeSlashCommand(
      invocation.command,
      SlashCommandContext(
        ref: ref,
        context: context,
        channelId: channelId,
        channel: channel,
        isDm: isDm,
        threadHeadId: threadHeadId,
        rootId: rootId,
        args: invocation.args,
        tokens: tokenizeSlashArgs(invocation.args),
        mentions: [
          for (final m in mentions) (pubkey: m.pubkey, label: m.label),
        ],
        channels: refs.channels,
        unresolvedChannelNames: refs.unresolved,
        sendMessage: (content, {mentionPubkeys = const []}) => send(
          content,
          mentionPubkeys,
          mediaTags: buildCustomEmojiTags(content, customEmoji),
        ),
        showMessage: (message) => messenger
          ?..hideCurrentSnackBar()
          ..showSnackBar(SnackBar(content: Text(message))),
      ),
    );
  }

  return (suggestions: suggestions, select: select, run: run);
}

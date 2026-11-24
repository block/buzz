part of '../thread_detail_page.dart';

class _ThreadComposer extends ConsumerWidget {
  const _ThreadComposer({
    required this.channelId,
    required this.channel,
    required this.head,
    required this.effectiveRootId,
    required this.focusNode,
    required this.onFocusRestorerChanged,
    required this.onFocusRequested,
    required this.prefillScopeMessages,
  });

  final String channelId;
  final Channel? channel;
  final TimelineMessage head;
  final String effectiveRootId;
  final FocusNode focusNode;
  final ValueChanged<VoidCallback> onFocusRestorerChanged;
  final VoidCallback onFocusRequested;
  final List<TimelineMessage> prefillScopeMessages;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sender = ref.read(sendMessageProvider);
    final queueText = ref.read(queueConversationTextProvider);
    final enabled = ref.watch(textOutboxSupportedProvider);
    final scope = enabled ? ref.watch(textOutboxScopeProvider) : null;
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (enabled)
          _ThreadOutbox(
            channelId: channelId,
            parentId: head.id,
            visibleEventIds: prefillScopeMessages.map((message) => message.id),
          ),
        ComposeBar(
          channelId: channelId,
          focusNode: focusNode,
          onFocusRestorerChanged: onFocusRestorerChanged,
          hintText: 'Reply in thread…',
          threadHeadId: head.id,
          rootId: effectiveRootId,
          prefillScopeMessages: channel?.isDm ?? true
              ? null
              : prefillScopeMessages,
          onFocusRequested: onFocusRequested,
          onQueueText: enabled
              ? (id, text, content, recipients, tags, {possiblySent}) =>
                    queueText(
                      scope: scope,
                      id: id,
                      channelId: channelId,
                      channel: channel,
                      text: text,
                      content: content,
                      recipients: recipients,
                      tags: tags,
                      parentId: head.id,
                      rootId: effectiveRootId,
                      possiblySent: possiblySent,
                    )
              : null,
          onSend: (content, mentionPubkeys, {mediaTags = const []}) =>
              sender.call(
                channelId: channelId,
                content: content,
                mentionPubkeys: mentionPubkeys,
                channel: channel,
                parentEventId: head.id,
                rootEventId: effectiveRootId,
                mediaTags: mediaTags,
              ),
        ),
      ],
    );
  }
}

class _ThreadOutbox extends ConsumerWidget {
  const _ThreadOutbox({
    required this.channelId,
    required this.parentId,
    required this.visibleEventIds,
  });

  final String channelId;
  final String parentId;
  final Iterable<String> visibleEventIds;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (!ref.watch(textOutboxSupportedProvider)) return const SizedBox.shrink();
    final scope = ref.watch(textOutboxScopeProvider);
    if (scope == null) return const SizedBox.shrink();
    return TextOutboxRows(
      scope: scope,
      channelId: channelId,
      parentId: parentId,
      visibleEventIds: visibleEventIds,
    );
  }
}

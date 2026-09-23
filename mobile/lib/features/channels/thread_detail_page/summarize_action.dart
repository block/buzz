part of '../thread_detail_page.dart';

/// Thread app-bar action that digests the thread the reader is looking at.
///
/// Deliberately scoped to the thread header rather than the channel header:
/// non-DM channels intentionally carry no app-bar actions (their actions live
/// behind the tappable title), and `channel_detail_page_test.dart` asserts that
/// absence.
class _SummarizeThreadButton extends ConsumerWidget {
  const _SummarizeThreadButton({
    required this.channelId,
    required this.messages,
  });

  final String channelId;

  /// The thread in reading order — head first, then replies.
  final List<TimelineMessage> messages;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return IconButton(
      key: const ValueKey('thread-summarize-button'),
      color: context.colors.primary,
      tooltip: 'Summarize thread',
      onPressed: () => unawaited(
        showThreadSummarySheet(
          context: context,
          ref: ref,
          channelId: channelId,
          messages: threadSummaryDigest(
            messages,
            // Read, not watch: the transcript is assembled once, on tap. A
            // watch here would rebuild the button on every profile that
            // trickles in from the kind:0 batch fetch.
            profiles: ref.read(userCacheProvider),
          ),
        ),
      ),
      icon: const Icon(LucideIcons.sparkles, size: 22),
    );
  }
}

/// Converts rendered thread messages into summarizer input.
///
/// System rows (joins, huddles, edits) are dropped: they are chrome around the
/// conversation, not part of it, and they would crowd out real content in a
/// digest capped at a handful of lines.
///
/// Author names resolve the same way the message rows resolve them — cached
/// profile label, else a shortened pubkey — so the digest names people the way
/// the thread above it does.
List<ThreadMessageDigest> threadSummaryDigest(
  List<TimelineMessage> messages, {
  required Map<String, UserProfile> profiles,
}) {
  return [
    for (final message in messages)
      if (!message.isSystem && message.content.trim().isNotEmpty)
        ThreadMessageDigest(
          author:
              profiles[message.pubkey.toLowerCase()]?.label ??
              shortPubkey(message.pubkey),
          text: message.content,
        ),
  ];
}

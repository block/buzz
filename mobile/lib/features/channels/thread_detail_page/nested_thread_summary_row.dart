part of '../thread_detail_page.dart';

/// Tappable summary row shown below a reply that itself has replies.
/// Pushes a new [ThreadDetailPage] for the nested thread.
class _NestedThreadSummaryRow extends ConsumerWidget {
  final ThreadSummary summary;
  final TimelineMessage replyMessage;
  final List<TimelineMessage> allMessages;
  final String channelId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;

  const _NestedThreadSummaryRow({
    required this.summary,
    required this.replyMessage,
    required this.allMessages,
    required this.channelId,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return Padding(
      padding: const EdgeInsets.only(
        left: messageAvatarSize + messageAvatarContentGap + Grid.xxs,
        bottom: Grid.xxs,
      ),
      child: ReplySummaryLink(
        key: ValueKey('nested-thread-summary-${replyMessage.id}'),
        summary: summary,
        hasReactions: replyMessage.reactions.isNotEmpty,
        onTap: () {
          Navigator.of(context).push(
            MaterialPageRoute<void>(
              builder: (_) => ThreadDetailPage(
                threadHead: replyMessage,
                allMessages: allMessages,
                channelId: channelId,
                currentPubkey: currentPubkey,
                isMember: isMember,
                isArchived: isArchived,
              ),
            ),
          );
        },
      ),
    );
  }
}

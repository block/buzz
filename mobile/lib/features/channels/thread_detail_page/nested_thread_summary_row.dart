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
    final userCache = ref.watch(userCacheProvider);

    return GestureDetector(
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
      child: Padding(
        key: ValueKey('nested-thread-summary-${replyMessage.id}'),
        padding: const EdgeInsets.only(
          left: messageAvatarSize + messageAvatarContentGap,
          top: Grid.half,
          bottom: Grid.xs,
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            // Stacked participant avatars.
            SizedBox(
              width:
                  32.0 +
                  (summary.participantPubkeys.length - 1).clamp(0, 2) * 20.0,
              height: 32,
              child: Stack(
                children: [
                  for (var i = 0; i < summary.participantPubkeys.length; i++)
                    Positioned(
                      left: i * 20.0,
                      child: SmallAvatar(
                        pubkey: summary.participantPubkeys[i],
                        userCache: userCache,
                        size: 32,
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(width: Grid.xxs),
            Flexible(
              child: Text.rich(
                TextSpan(
                  children: [
                    TextSpan(
                      text:
                          '${summary.replyCount} ${summary.replyCount == 1 ? 'reply' : 'replies'}',
                      style: replyPreviewTextStyle.copyWith(
                        color: context.colors.primary,
                      ),
                    ),
                    if (summary.lastReplyAt case final lastReplyAt?) ...[
                      TextSpan(
                        text: ' · ',
                        style: replyPreviewTextStyle.copyWith(
                          color: context.colors.onSurfaceVariant.withValues(
                            alpha: 0.5,
                          ),
                        ),
                      ),
                      TextSpan(
                        text:
                            'last reply ${formatThreadSummaryLastReplyTime(lastReplyAt)}',
                        style: replyPreviewTextStyle.copyWith(
                          color: context.colors.onSurfaceVariant,
                        ),
                      ),
                    ],
                  ],
                ),
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

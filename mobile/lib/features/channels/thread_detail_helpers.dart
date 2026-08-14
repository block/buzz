part of 'thread_detail_page.dart';

int _threadTailIndex(int replyCount) => replyCount;

void _resumeThreadTailFollow({
  required bool Function() isVisible,
  required ObjectRef<bool> userOptedOut,
  required ObjectRef<bool> followsTail,
}) {
  if (!isVisible()) return;
  userOptedOut.value = false;
  followsTail.value = true;
}

bool _isDeletedBy(Iterable<NostrEvent> events, String messageId) {
  for (final event in events) {
    if (event.kind != EventKind.deletion &&
        event.kind != EventKind.nip29DeleteEvent) {
      continue;
    }
    if (event.tags.any(
      (tag) => tag.length >= 2 && tag[0] == 'e' && tag[1] == messageId,
    )) {
      return true;
    }
  }
  return false;
}

/// Build a lightweight summary for a nested thread (reply that has its own
/// replies). Same logic as the top-level [ThreadSummary] but kept local to
/// avoid coupling.
ThreadSummary _buildNestedSummary(
  String messageId,
  List<TimelineMessage> children,
) {
  final seen = <String>{};
  final participants = <String>[];
  for (var i = children.length - 1; i >= 0 && participants.length < 3; i--) {
    final pk = children[i].pubkey.toLowerCase();
    if (seen.add(pk)) participants.add(pk);
  }
  return ThreadSummary(
    threadHeadId: messageId,
    replyCount: children.length,
    participantPubkeys: participants.reversed.toList(),
    lastReplyAt: children.last.createdAt,
  );
}

/// Serializes deferred tail work behind the latest user scroll intent.
class _ThreadTailIntent {
  var _generation = 0;
  var isDragging = false;

  void detach() => _generation++;

  void beginDrag() {
    isDragging = true;
    detach();
  }

  void endDrag() => isDragging = false;

  void scheduleNextFrame({
    required bool allowed,
    required bool Function() revalidate,
    required VoidCallback action,
  }) {
    if (!allowed) return;
    final generation = ++_generation;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (generation == _generation && revalidate()) action();
    });
  }

  void schedule({
    required bool allowed,
    required bool Function() revalidate,
    required VoidCallback action,
  }) {
    if (!allowed) return;
    final generation = ++_generation;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (generation == _generation && revalidate()) action();
      });
      WidgetsBinding.instance.scheduleFrame();
    });
  }
}

/// Thread-scoped typing status with optional size animation.
class _ThreadTypingIndicator extends StatelessWidget {
  final List<TypingEntry> entries;
  final bool animated;

  const _ThreadTypingIndicator({required this.entries, this.animated = true});

  @override
  Widget build(BuildContext context) {
    final child = entries.isEmpty
        ? const SizedBox.shrink()
        : ChannelTypingIndicator(entries: entries);
    if (!animated || MediaQuery.disableAnimationsOf(context)) return child;
    return AnimatedSize(
      duration: const Duration(milliseconds: 180),
      curve: Curves.easeOutCubic,
      alignment: Alignment.bottomCenter,
      child: child,
    );
  }
}

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

class _Avatar extends StatelessWidget {
  final UserProfile? profile;
  final String pubkey;

  const _Avatar({required this.profile, required this.pubkey});

  @override
  Widget build(BuildContext context) {
    final initial =
        profile?.initial ?? (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?');
    final avatarUrl = profile?.avatarUrl;

    return AvatarImage(
      imageUrl: avatarUrl,
      radius: messageAvatarSize / 2,
      backgroundColor: context.colors.primaryContainer,
      fallback: Text(
        initial,
        style: context.textTheme.labelMedium?.copyWith(
          color: context.colors.onPrimaryContainer,
          fontWeight: FontWeight.w600,
        ),
      ),
    );
  }
}

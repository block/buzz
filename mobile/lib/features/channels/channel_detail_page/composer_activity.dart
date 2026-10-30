part of '../channel_detail_page.dart';

/// Desktop's conversation activity above the channel composer. Its rows open
/// the threads the user's agents work in, except in Me, which has none.
class _ComposerActivity extends StatelessWidget {
  const _ComposerActivity({
    required this.scope,
    required this.channel,
    required this.messages,
  });

  final ActivityScope scope;
  final Channel channel;
  final List<TimelineMessage> messages;

  void _openThread(BuildContext context, String rootId) {
    // The sheet closed a frame ago; this page may have gone meanwhile.
    if (!context.mounted) return;
    final root = messages
        .where((message) => message.id.toLowerCase() == rootId.toLowerCase())
        .firstOrNull;
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => root == null
            // As desktop opens a thread by its link, load a root that is not
            // in the timeline, then show its thread.
            ? ChannelDetailPage(
                channel: channel,
                initialThreadRootId: rootId,
                initialThreadRouteBehavior:
                    InitialThreadRouteBehavior.replaceCurrentRoute,
              )
            : ThreadDetailPage(
                threadHead: root,
                allMessages: messages,
                channelId: scope.channelId,
                currentPubkey: scope.selfPubkey,
                isMember: channel.isMember,
                isArchived: channel.isArchived,
              ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final child = ConversationActivity(
      scope: scope,
      messages: messages,
      onOpenThread: scope.allThreads
          ? null
          : (rootId) => _openThread(context, rootId),
    );
    if (MediaQuery.disableAnimationsOf(context)) return child;
    return AnimatedSize(
      duration: const Duration(milliseconds: 180),
      curve: Curves.easeOutCubic,
      alignment: Alignment.bottomCenter,
      child: child,
    );
  }
}

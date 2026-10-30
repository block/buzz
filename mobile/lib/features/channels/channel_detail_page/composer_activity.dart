part of '../channel_detail_page.dart';

/// Desktop's conversation activity above the channel composer. Its rows open
/// the threads the user's agents work in.
class _ComposerActivity extends StatelessWidget {
  const _ComposerActivity({
    required this.scope,
    required this.messages,
    required this.isMember,
    required this.isArchived,
  });

  final ActivityScope scope;
  final List<TimelineMessage> messages;
  final bool isMember;
  final bool isArchived;

  @override
  Widget build(BuildContext context) {
    final child = ConversationActivity(
      scope: scope,
      messages: messages,
      onOpenThread: (root) => Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ThreadDetailPage(
            threadHead: root,
            allMessages: messages,
            channelId: scope.channelId,
            currentPubkey: scope.selfPubkey,
            isMember: isMember,
            isArchived: isArchived,
          ),
        ),
      ),
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

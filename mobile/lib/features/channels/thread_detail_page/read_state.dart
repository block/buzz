part of '../thread_detail_page.dart';

void _useThreadReplyReadState(
  WidgetRef ref,
  String threadHeadId,
  List<TimelineMessage> replies,
) {
  final readState = ref.watch(readStateProvider);
  final visibleReplyReadKey = replies
      .map((reply) => '${reply.id}:${reply.createdAt}')
      .join(',');

  useEffect(() {
    if (!readState.isReady || replies.isEmpty) return null;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      for (final reply in replies) {
        ref
            .read(readStateProvider.notifier)
            .markContextRead(msgContextKey(reply.id), reply.createdAt);
      }
    });
    return null;
  }, [threadHeadId, readState.isReady, visibleReplyReadKey]);
}

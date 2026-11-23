part of '../channel_detail_page.dart';

extension on _MessageList {
  void _useInitialThreadRoute(BuildContext context, WidgetRef ref) {
    final opened = useRef(false);
    final rootId = initialThreadRootId;
    final channelHead = rootId == null
        ? null
        : allMessages.where((message) => message.id == rootId).firstOrNull;
    final preview = rootId == null || channelHead != null || opened.value
        ? null
        : ref.watch(
            savedThreadPreviewProvider((channelId: channelId, rootId: rootId)),
          );
    final saved = preview?.isLoading == false ? preview?.value : null;
    final live = saved == null
        ? null
        : ref.watch(channelMessagesProvider(channelId)).value;
    final head = useMemoized(() {
      if (channelHead != null || saved == null) return channelHead;
      return formatTimeline(
        mergeThreadEvents(saved, live ?? const []),
        currentPubkey: currentPubkey,
      ).where((message) => message.id == rootId).firstOrNull;
    }, [channelHead, saved, live, currentPubkey, rootId]);
    useEffect(() {
      if (opened.value || head == null) return null;
      var cancelled = false;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (cancelled ||
            !context.mounted ||
            ModalRoute.of(context)?.isCurrent != true) {
          return;
        }
        final error = ref.read(channelMessagesProvider(channelId)).error;
        if (error != null && isRelayChannelAccessDenied(error)) return;
        if (channelHead == null && rootId != null) {
          final current = ref.read(
            savedThreadPreviewProvider((channelId: channelId, rootId: rootId)),
          );
          if (current.isLoading || !identical(current.value, saved)) return;
        }
        opened.value = true;
        final route = MaterialPageRoute<void>(
          builder: (_) => ThreadDetailPage(
            threadHead: head,
            headFromSavedPage: channelHead == null,
            allMessages: allMessages,
            channelId: channelId,
            currentPubkey: currentPubkey,
            isMember: isMember,
            isArchived: isArchived,
            initialMessageId: initialMessageId,
          ),
        );
        final navigator = Navigator.of(context);
        switch (initialThreadRouteBehavior) {
          case InitialThreadRouteBehavior.push:
            navigator.push(route);
          case InitialThreadRouteBehavior.replaceCurrentRoute:
            navigator.pushReplacement(route);
        }
      });
      return () => cancelled = true;
    }, [head, channelHead, allMessages, initialThreadRouteBehavior]);
  }
}

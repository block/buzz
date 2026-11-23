part of '../channel_detail_page.dart';

extension on _MessageList {
  /// Brings a message into view when a page above returns here for it, such
  /// as a thread going up to its conversation. A message already on screen
  /// stays where the reader left it.
  void _useMessageReveal(
    BuildContext context,
    WidgetRef ref, {
    required ItemScrollController controller,
    required ValueListenable<Iterable<ItemPosition>> positions,
    required int? Function(String? messageId) indexOf,
    required double Function() viewportHeight,
    required double bottomInset,
    required VoidCallback onJump,
  }) {
    final request = useState<({String id, int serial})?>(null);
    final handled = useRef(0);
    final retained = useRef(<String>{});
    useOpenChannelReveal(ref, (messageId) {
      request.value = (id: messageId, serial: (request.value?.serial ?? 0) + 1);
      // The message can be older than the loaded window. Asking again for
      // one already held costs nothing, and retries one that failed.
      retained.value.add(messageId);
      unawaited(_loadDeepLinkEvents(notifier, {messageId}));
    });
    useEffect(
      () => () {
        notifier.releaseDeepLinkEvents(retained.value);
        retained.value = {};
      },
      [notifier],
    );

    final pending = request.value;
    final index = indexOf(pending?.id);
    final headerHeight = _conversationHeaderHeight(
      context,
      titleContentHeight: appBarTitleContentHeight,
    );
    useEffect(() {
      if (pending == null || index == null) return null;
      if (handled.value == pending.serial) return null;
      var cancelled = false;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (cancelled || !context.mounted || !controller.isAttached) return;
        handled.value = pending.serial;
        final height = viewportHeight();
        // This reversed list measures edges up from the viewport's bottom.
        final shown =
            height > 0 &&
            positions.value.any(
              (position) =>
                  position.index == index &&
                  position.itemLeadingEdge < 1 - headerHeight / height &&
                  position.itemTrailingEdge > bottomInset / height,
            );
        if (shown) return;
        onJump();
        controller.jumpTo(index: index, alignment: 0.35);
      });
      return () => cancelled = true;
    }, [pending?.serial, index]);
  }
}

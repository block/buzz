part of '../channel_detail_page.dart';

// Owns pagination for one status-bar gesture without changing list layout.
void _useChannelScrollToOldest(
  BuildContext context, {
  required ChannelMessagesNotifier notifier,
  required ItemScrollController itemScrollController,
  required int displayCount,
  required ValueNotifier<bool> isLoadingOlder,
  required ObjectRef<bool> navigatingOldest,
  required ObjectRef<bool> olderFetchFailed,
  required ObjectRef<int> oldestNavigationGeneration,
  required VoidCallback onStart,
}) {
  final currentDisplayCount = useRef(displayCount);
  currentDisplayCount.value = displayCount;
  Future<void> navigateToOldest() async {
    if (navigatingOldest.value ||
        !itemScrollController.isAttached ||
        displayCount == 0) {
      return;
    }
    navigatingOldest.value = true;
    olderFetchFailed.value = false;
    final generation = ++oldestNavigationGeneration.value;
    bool isCurrent() =>
        navigatingOldest.value &&
        oldestNavigationGeneration.value == generation;
    var ownsFetch = false;
    onStart();
    try {
      // Wait for an already-running edge fetch before owning pagination.
      while (isLoadingOlder.value && context.mounted && isCurrent()) {
        await WidgetsBinding.instance.endOfFrame;
      }
      while (context.mounted && isCurrent() && !notifier.reachedOldest) {
        isLoadingOlder.value = true;
        ownsFetch = true;
        final loaded = await notifier.fetchOlder();
        if (!context.mounted) return;
        isLoadingOlder.value = false;
        ownsFetch = false;
        if (!loaded) {
          olderFetchFailed.value = !notifier.reachedOldest;
          break;
        }
        await WidgetsBinding.instance.endOfFrame;
      }
      if (!context.mounted ||
          !isCurrent() ||
          !itemScrollController.isAttached) {
        return;
      }
      final index = currentDisplayCount.value - 1;
      if (index < 0) return;
      if (MediaQuery.disableAnimationsOf(context)) {
        itemScrollController.jumpTo(index: index);
      } else {
        await itemScrollController.scrollTo(
          index: index,
          duration: const Duration(milliseconds: 300),
          curve: Curves.easeOutCubic,
        );
      }
    } finally {
      if (oldestNavigationGeneration.value == generation) {
        navigatingOldest.value = false;
      }
      if (context.mounted && ownsFetch) isLoadingOlder.value = false;
    }
  }

  useIosScrollToTop(context, () => unawaited(navigateToOldest()));
}

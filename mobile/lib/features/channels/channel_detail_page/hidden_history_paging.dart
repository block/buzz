part of '../channel_detail_page.dart';

/// Pages fetched without user input while every loaded event is a hidden
/// membership notice. Past this, the list offers a button instead.
const _maxHiddenOnlyAutoPages = 5;

/// A loaded page can hold only hidden self-joins and leaves, leaving no row to
/// scroll for older history. While [active], pages older history automatically
/// until something renders, at most [_maxHiddenOnlyAutoPages] times, and
/// returns what to show in place of the empty list: a loading indicator, or a
/// button that resumes paging after the cap or a failed fetch. Null when not
/// [active].
Widget? _useHiddenHistoryPaging({
  required BuildContext context,
  required bool active,
  required ChannelMessagesNotifier notifier,
  required int loadedCount,
  required ValueNotifier<bool> isLoadingOlder,
  required ObjectRef<bool> olderFetchPaused,
}) {
  final autoPages = useRef(0);
  final resume = useState(0);
  if (!active) autoPages.value = 0;
  final capped = autoPages.value >= _maxHiddenOnlyAutoPages;
  useEffect(() {
    if (!active || capped || isLoadingOlder.value || olderFetchPaused.value) {
      return null;
    }
    var cancelled = false;
    // After this build: fetching updates providers and this widget's state.
    unawaited(
      Future<void>(() async {
        if (cancelled || !context.mounted) return;
        autoPages.value++;
        isLoadingOlder.value = true;
        try {
          final loaded = await notifier.fetchOlder();
          if (context.mounted) {
            olderFetchPaused.value = !loaded && !notifier.reachedOldest;
          }
        } finally {
          if (context.mounted) isLoadingOlder.value = false;
        }
      }),
    );
    return () => cancelled = true;
  }, [active, loadedCount, isLoadingOlder.value, resume.value]);

  if (!active) return null;
  if (isLoadingOlder.value || (!capped && !olderFetchPaused.value)) {
    return const Center(
      child: BuzzLoadingIndicator(
        size: 24,
        semanticLabel: 'Loading older messages',
      ),
    );
  }
  return Center(
    child: TextButton(
      onPressed: () {
        autoPages.value = 0;
        olderFetchPaused.value = false;
        resume.value++;
      },
      child: const Text('Load earlier messages'),
    ),
  );
}

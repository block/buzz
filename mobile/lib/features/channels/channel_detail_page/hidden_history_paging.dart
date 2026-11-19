part of '../channel_detail_page.dart';

/// Older pages fetched without user input, while every loaded event is a
/// hidden membership notice or the loaded rows fit the viewport. Past this,
/// the list offers a button instead.
const _maxAutoOlderPages = 5;

/// A loaded page can hold only hidden self-joins and leaves, leaving no row to
/// scroll for older history. While nothing renders ([noRows]) but hidden
/// notices are [loaded] and older history remains, pages older history
/// automatically, at most [_maxAutoOlderPages] times. Returns what to show in
/// place of the empty list: a loading indicator, or a button that resumes
/// paging after the cap or a failed fetch. Null otherwise.
Widget? _useHiddenHistoryPaging({
  required BuildContext context,
  required bool noRows,
  required List<TimelineMessage> loaded,
  required ChannelMessagesNotifier notifier,
  required ValueNotifier<bool> isLoadingOlder,
  required ObjectRef<bool> olderFetchPaused,
}) {
  final active =
      noRows && !notifier.reachedOldest && loaded.any(isHiddenMembershipNotice);
  final autoPages = useRef(0);
  final resume = useState(0);
  if (!active) autoPages.value = 0;
  final capped = autoPages.value >= _maxAutoOlderPages;
  useEffect(() {
    if (!active || capped || isLoadingOlder.value || olderFetchPaused.value) {
      return null;
    }
    var cancelled = false;
    // After this build: fetching updates providers and this widget's state.
    unawaited(
      Future<void>(() {
        if (cancelled || !context.mounted) return;
        autoPages.value++;
        _fetchOlderPage(context, notifier, isLoadingOlder, olderFetchPaused);
      }),
    );
    return () => cancelled = true;
  }, [active, loaded.length, isLoadingOlder.value, resume.value]);

  if (!active) return null;
  if (isLoadingOlder.value || (!capped && !olderFetchPaused.value)) {
    return const Center(
      child: BuzzLoadingIndicator(
        size: 24,
        semanticLabel: 'Loading older messages',
      ),
    );
  }
  return _LoadEarlierButton(
    onPressed: () {
      autoPages.value = 0;
      olderFetchPaused.value = false;
      resume.value++;
    },
  );
}

/// Keeps the [hiddenHistory] spinner or button above [child], so another
/// empty-state widget doesn't hide the way to older history.
Widget _aboveHiddenHistory(Widget? hiddenHistory, Widget child) =>
    hiddenHistory == null
    ? child
    : Column(mainAxisSize: MainAxisSize.min, children: [hiddenHistory, child]);

/// Resumes paging older history when automatic paging stopped.
class _LoadEarlierButton extends StatelessWidget {
  const _LoadEarlierButton({required this.onPressed});

  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => Center(
    child: TextButton(
      onPressed: onPressed,
      child: const Text('Load earlier messages'),
    ),
  );
}

/// Whether the oldest loaded row is wholly on screen, leaving nothing to
/// scroll toward older history.
bool _oldestRowFits(Iterable<ItemPosition> positions, int rowCount) =>
    positions.any(
      (position) =>
          position.index == rowCount - 1 && position.itemTrailingEdge <= 1,
    );

/// History that fits the viewport can't be scrolled (Android clamping), so
/// while the loaded rows [fits] the viewport and the user hasn't [scrolled],
/// older pages are fetched automatically ([fills]). Every automatic fetch
/// since the last scroll gesture counts toward [_maxAutoOlderPages]; at the
/// cap, or after a [paused] failure, the list offers a button.
({bool fills, bool offerLoadEarlier}) _viewportFill({
  required bool fits,
  required bool scrolled,
  required bool busy,
  required int pages,
  required bool reachedOldest,
  required bool paused,
}) {
  final fills = !scrolled && !busy && fits && pages < _maxAutoOlderPages;
  return (
    fills: fills,
    offerLoadEarlier:
        fits &&
        !reachedOldest &&
        !busy &&
        (paused || pages >= _maxAutoOlderPages),
  );
}

/// Fetches one older page. A failed or thrown fetch is recorded as a pause,
/// which an explicit request or a scroll clears.
void _fetchOlderPage(
  BuildContext context,
  ChannelMessagesNotifier notifier,
  ValueNotifier<bool> isLoadingOlder,
  ObjectRef<bool> olderFetchPaused, {
  VoidCallback? onDone,
}) {
  olderFetchPaused.value = false;
  isLoadingOlder.value = true;
  notifier
      .fetchOlder()
      .then(
        (loaded) => olderFetchPaused.value = !loaded && !notifier.reachedOldest,
        onError: (Object _) => olderFetchPaused.value = true,
      )
      .whenComplete(() {
        if (!context.mounted) return;
        isLoadingOlder.value = false;
        onDone?.call();
      });
}

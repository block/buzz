part of '../compose_bar.dart';

/// The rows the open mention chooser shows, and the shown rows that can no
/// longer be chosen. A row keeps its place while the query and the opening
/// stay the same (portable mention rules, section 6).
///
/// The chooser sits on the composer and grows upward, so a taller chooser
/// moves every shown row. While the directory search can still add people
/// for this query, `reserveHeight` holds the chooser at full height so a
/// late row fills empty space instead of moving the row under a finger.
/// The reservation lasts until the query or the opening changes.
/// `searching` is true only while that search is still running.
typedef _MentionRows = ({
  List<MentionCandidate> rows,
  Set<String> unavailable,
  bool searchFailed,
  bool reserveHeight,
  bool searching,
});

_MentionRows _useMentionRows(
  WidgetRef ref, {
  required String channelId,
  required String? query,
  required int opening,
}) {
  final args = query == null
      ? null
      : (channelId: channelId, query: query, opening: opening);
  final ranked = args == null
      ? const <MentionCandidate>[]
      : ref.watch(mentionCandidatesProvider(args));
  final searchFailed =
      args != null && ref.watch(mentionSearchFailedProvider(args));
  final pending = args != null && ref.watch(mentionSearchPendingProvider(args));
  final shown = useStableMentionRows(
    ranked,
    query: query ?? '',
    opening: opening,
    pending: pending,
  );
  if (query == null) {
    return (
      rows: const [],
      unavailable: const {},
      searchFailed: false,
      reserveHeight: false,
      searching: false,
    );
  }
  return (
    rows: shown.rows,
    unavailable: shown.unavailable,
    searchFailed: searchFailed,
    reserveHeight: shown.reserveHeight,
    searching: pending,
  );
}

/// Section 5: the shown row that Space selects after [query], or null. The
/// exact-name check uses every ranked choice, not only the capped rows.
MentionCandidate? _spaceMention(
  WidgetRef ref, {
  required String channelId,
  required String query,
  required int opening,
  required _MentionRows shown,
  required String? viewer,
}) {
  final ranked = ref.read(
    mentionCandidatesProvider((
      channelId: channelId,
      query: query,
      opening: opening,
    )),
  );
  final key = exactMention([
    for (final candidate in ranked) mentionChoiceOf(candidate, viewer),
  ], query);
  if (key == null || shown.unavailable.contains(key)) return null;
  return shown.rows.where((row) => row.pubkey == key).firstOrNull;
}

const _noMentionRows = (
  rows: <MentionCandidate>[],
  unavailable: <String>{},
  searchFailed: false,
  reserveHeight: false,
  searching: false,
);

/// One editor's `@` chooser: the open query, its shown rows, and the chip a
/// pick or a typed terminator inserts. The composer and the message edit
/// sheet share it, so both follow the same trigger, ranking, search,
/// selection and binding rules.
class _MentionChooser {
  /// The open query after its `@`, or null while no chooser is open, as
  /// of this build.
  final String? query;

  /// The query as it is now, ahead of the next frame: a hardware key can
  /// arrive after an edit changed the query but before its rows rendered.
  final ValueListenable<String?> liveQuery;
  final _MentionRows rows;

  /// Replaces the open query with a chip bound to [candidate], followed by
  /// [separator]: a space, or the closing punctuation that selected it.
  final void Function(MentionCandidate candidate, {String separator}) insert;

  /// Closes the chooser; the text is untouched.
  final VoidCallback close;

  /// Follows one editor change. Returns true while a query is open after
  /// it, or when the typed character selected a name and the text changed;
  /// false when the caret is not at a query.
  final bool Function(TextEditingValue previous, TextEditingValue current)
  track;

  const _MentionChooser({
    required this.query,
    required this.liveQuery,
    required this.rows,
    required this.insert,
    required this.close,
    required this.track,
  });
}

_MentionChooser _useMentionChooser(
  WidgetRef ref, {
  required String channelId,
  required _MarkdownEditingController controller,
  required FocusNode focusNode,
  required ObjectRef<bool> isModifyingText,
  required String? viewer,
}) {
  final query = useState<String?>(null);
  final startIdx = useState(-1);
  // Counts chooser openings: a new `@` starts fresh rows and searches.
  final opening = useRef(0);
  final shown = useRef<_MentionRows>(_noMentionRows);
  final viewerRef = useRef(viewer)..value = viewer;

  void insert(MentionCandidate candidate, {String separator = ' '}) {
    ref
        .read(mentionHistoryProvider.notifier)
        .remember(channelId, candidate.pubkey);
    final text = controller.text;
    final start = startIdx.value.clamp(0, text.length);
    final cursor =
        (controller.selection.isValid
                ? controller.selection.baseOffset
                : text.length)
            .clamp(start, text.length);
    isModifyingText.value = true;
    try {
      controller.insertMention(
        start: start,
        end: cursor,
        candidate: candidate,
        separator: separator,
      );
    } finally {
      isModifyingText.value = false;
    }
    focusNode.requestFocus();
    query.value = null;
  }

  bool track(TextEditingValue previous, TextEditingValue current) {
    final text = current.text;
    final sel = current.selection;
    if (!sel.isValid || !sel.isCollapsed || sel.baseOffset < 1) {
      query.value = null;
      return false;
    }
    var open = findMentionQuery(text, sel.baseOffset);
    if (open != null &&
        _startsSelectedMention(controller.bindings, open.start)) {
      open = null;
    }

    // Space or closing punctuation typed straight after an exact, unique
    // name selects it (section 5); the typed character stays after the
    // chip. Otherwise the character joins the query.
    final previousQuery = query.value;
    if (open != null &&
        previousQuery != null &&
        previousQuery.trim().isNotEmpty &&
        open.start == startIdx.value &&
        open.query.length == previousQuery.length + 1 &&
        open.query.startsWith(previousQuery) &&
        text.length == previous.text.length + 1) {
      final typed = open.query.substring(previousQuery.length);
      final chosen = mentionTerminator(typed)
          ? _spaceMention(
              ref,
              channelId: channelId,
              query: previousQuery,
              opening: opening.value,
              shown: shown.value,
              viewer: viewerRef.value,
            )
          : null;
      if (chosen != null) {
        insert(chosen, separator: typed);
        return true;
      }
    }

    if (open == null) {
      query.value = null;
      return false;
    }
    if (previousQuery == null || startIdx.value != open.start) {
      opening.value++;
    }
    query.value = open.query;
    startIdx.value = open.start;
    return true;
  }

  // Ranked rows; shown rows hold still while the query stays.
  final rows = _useMentionRows(
    ref,
    channelId: channelId,
    query: query.value,
    opening: opening.value,
  );
  shown.value = rows;

  // Resolve owner names for the visible "managed by …" subtitles.
  useEffect(() {
    final ownerPubkeys = [for (final s in rows.rows) ?s.ownerPubkey];
    if (ownerPubkeys.isNotEmpty) {
      ref.read(userCacheProvider.notifier).preload(ownerPubkeys);
    }
    return null;
  }, [rows.rows.length, query.value]);

  return _MentionChooser(
    query: query.value,
    liveQuery: query,
    rows: rows,
    insert: insert,
    close: () => query.value = null,
    track: track,
  );
}

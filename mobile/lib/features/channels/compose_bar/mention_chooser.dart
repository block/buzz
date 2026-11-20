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

/// Whether [caret] is inside a mention the user already selected at [start].
/// Such an `@` does not open a query.
bool _insideSelectedMention(
  String text,
  int start,
  int caret,
  Iterable<String> names,
) => names.any(
  (name) =>
      text.startsWith('@$name', start) && caret <= start + 1 + name.length,
);

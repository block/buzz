part of '../compose_bar.dart';

/// The rows the open mention chooser shows, and the shown rows that can no
/// longer be chosen. A row keeps its place while the query and the opening
/// stay the same (portable mention rules, section 6).
typedef _MentionRows = ({
  List<MentionCandidate> rows,
  Set<String> unavailable,
  bool searchFailed,
});

_MentionRows _useMentionRows(
  WidgetRef ref, {
  required String channelId,
  required String? query,
  required int opening,
}) {
  final shown = useRef<(String, int, List<MentionCandidate>)?>(null);
  if (query == null) {
    shown.value = null;
    return (rows: const [], unavailable: const {}, searchFailed: false);
  }
  final args = (channelId: channelId, query: query, opening: opening);
  final ranked = ref.watch(mentionCandidatesProvider(args));
  final searchFailed = ref.watch(mentionSearchFailedProvider(args));
  final last = shown.value;
  final same = last != null && last.$1 == query && last.$2 == opening;
  final result = stableMentionRows(same ? last.$3 : const [], ranked);
  shown.value = (query, opening, result.rows);
  return (
    rows: result.rows,
    unavailable: result.unavailable,
    searchFailed: searchFailed,
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

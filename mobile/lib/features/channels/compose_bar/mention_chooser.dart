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
  final shown = useRef<(String, int, List<MentionCandidate>, bool)?>(null);
  if (query == null) {
    shown.value = null;
    return (
      rows: const [],
      unavailable: const {},
      searchFailed: false,
      reserveHeight: false,
      searching: false,
    );
  }
  final args = (channelId: channelId, query: query, opening: opening);
  final ranked = ref.watch(mentionCandidatesProvider(args));
  final searchFailed = ref.watch(mentionSearchFailedProvider(args));
  final pending = ref.watch(mentionSearchPendingProvider(args));
  final last = shown.value;
  final same = last != null && last.$1 == query && last.$2 == opening;
  final result = stableMentionRows(same ? last.$3 : const [], ranked);
  final reserveHeight = pending || (same && last.$4);
  shown.value = (query, opening, result.rows, reserveHeight);
  return (
    rows: result.rows,
    unavailable: result.unavailable,
    searchFailed: searchFailed,
    reserveHeight: reserveHeight,
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

/// Whether the `@` at [start] begins a mention the user already selected.
/// Such an `@` does not open a query, wherever the caret is: text after a
/// selected mention is prose, so a later `#` can open channel suggestions.
/// A label followed by more name characters is a different, typed name.
bool _startsSelectedMention(String text, int start, Iterable<String> names) =>
    names.any((name) {
      if (!text.startsWith('@$name', start)) return false;
      final end = start + 1 + name.length;
      return end == text.length || !_nameContinuation.hasMatch(text[end]);
    });

final _nameContinuation = RegExp(r'[\p{L}\p{N}_]', unicode: true);

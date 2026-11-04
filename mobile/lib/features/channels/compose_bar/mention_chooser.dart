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

/// The chooser rows for [query]. With [dead], a multi-word query at [start]
/// that settles with no rows is recorded there (see [_settledDeadMention]).
_MentionRows _useMentionRows(
  WidgetRef ref, {
  required String channelId,
  required String? query,
  required int opening,
  int start = -1,
  ObjectRef<_DeadMention?>? dead,
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
  final rows = (
    rows: shown.rows,
    unavailable: shown.unavailable,
    searchFailed: searchFailed,
    reserveHeight: shown.reserveHeight,
    searching: pending,
  );
  if (dead != null) {
    dead.value = _settledDeadMention(dead.value, query, start, rows);
  }
  return rows;
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

/// The span an edit from [before] to [after] replaced: text before
/// `prefix` and from `beforeEnd` in [before] (`afterEnd` in [after]) on is
/// unchanged.
({int prefix, int beforeEnd, int afterEnd}) _editSpan(
  String before,
  String after,
) {
  final shorter = math.min(before.length, after.length);
  var prefix = 0;
  while (prefix < shorter &&
      before.codeUnitAt(prefix) == after.codeUnitAt(prefix)) {
    prefix++;
  }
  var suffix = 0;
  while (suffix < shorter - prefix &&
      before.codeUnitAt(before.length - 1 - suffix) ==
          after.codeUnitAt(after.length - 1 - suffix)) {
    suffix++;
  }
  return (
    prefix: prefix,
    beforeEnd: before.length - suffix,
    afterEnd: after.length - suffix,
  );
}

/// Moves each text offset in [offsets] across the edit from [before] to
/// [after]: offsets before the edited span stay, offsets after it shift by
/// the length change, and offsets inside it are dropped.
void _rebaseOffsets(Set<int> offsets, String before, String after) {
  if (offsets.isEmpty || before == after) return;
  final span = _editSpan(before, after);
  final moved = {
    for (final offset in offsets)
      if (offset < span.prefix)
        offset
      else if (offset >= span.beforeEnd)
        offset + after.length - before.length,
  };
  offsets
    ..clear()
    ..addAll(moved);
}

/// Whether the edit from [before] to [after] inserted the character at
/// [index] of [after], so an `@` there was pasted or typed just now.
bool _insertedByEdit(String before, String after, int index) {
  final span = _editSpan(before, after);
  return index >= span.prefix && index < span.afterEnd;
}

/// Moves the [typed] `@` offsets and the [dead] query across the edit from
/// [before] to [after]; every edit, including chip insertion, passes here.
/// [dead] moves with its `@` when the edit is wholly before it, and drops
/// when the edit replaced that `@` or no longer leaves the dead query.
void _rebaseMentions(
  ObjectRef<Set<int>> typed,
  ObjectRef<_DeadMention?> dead,
  String before,
  String after,
) {
  _rebaseOffsets(typed.value, before, after);
  final current = dead.value;
  if (current == null || before == after) return;
  final span = _editSpan(before, after);
  final next = span.prefix > current.start
      ? current
      : span.beforeEnd > current.start
      ? null
      : (
          start: current.start + after.length - before.length,
          query: current.query,
        );
  // Shortening or changing the dead text revives the query.
  dead.value =
      next != null &&
          after.substring(next.start).toLowerCase().startsWith('@${next.query}')
      ? next
      : null;
}

/// Whether every source of mention choices has loaded, so a query with no
/// rows has truly settled: channel members and the agent directories.
bool _mentionSourcesSettled(
  WidgetRef ref,
  AsyncValue<List<ChannelMember>> members,
) =>
    members.hasValue &&
    !ref.watch(agentDirectoryProvider).isLoading &&
    !ref.watch(agentOwnersProvider).isLoading &&
    !ref.watch(ownedAgentDirectoryProvider).isLoading;

/// The `@` position and lowercased query of a query that settled as prose.
typedef _DeadMention = ({int start, String query});

/// [open], or null when that mention query should stay closed.
///
/// A selected mention's `@` does not reopen a query. One that already opened
/// a query ([typed]) stays open, and so does one this edit inserted (a
/// paste), so a new `@Alice Smith` can still reach "Alice Smith" when an
/// earlier `@Alice` was selected. A query that already settled as prose
/// ([dead]) stays closed while it grows.
MentionQuery? _openMentionQuery(
  MentionQuery? open, {
  required String before,
  required String text,
  required Set<int> typed,
  required Iterable<String> selected,
  required _DeadMention? dead,
}) {
  if (open == null) return null;
  final selectedAt =
      !typed.contains(open.start) &&
      !_insertedByEdit(before, text, open.start) &&
      _startsSelectedMention(text, open.start, selected);
  final prose =
      dead != null &&
      open.start == dead.start &&
      open.query.toLowerCase().startsWith(dead.query);
  return selectedAt || prose ? null : open;
}

/// The dead query after [rows] settle for [query] at [start].
///
/// A multi-word query that settled with no choices is prose (portable mention
/// rules, section 2). Adding to it cannot find a name, so it stays closed,
/// and searches nothing, until the user shortens it. A query still searching
/// or with a failed search has not settled. Callers pass no dead-query ref
/// until every choice source loads ([_mentionSourcesSettled]), since rows
/// from a partial roster settle nothing.
_DeadMention? _settledDeadMention(
  _DeadMention? current,
  String query,
  int start,
  _MentionRows rows,
) {
  if (!query.trim().contains(' ') ||
      rows.rows.isNotEmpty ||
      rows.searching ||
      rows.searchFailed) {
    return current;
  }
  return (start: start, query: query.toLowerCase());
}

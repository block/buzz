import 'package:flutter_hooks/flutter_hooks.dart';

import 'mention_ranking.dart';

/// The rows a mention-style chooser shows for [ranked], and the shown rows
/// that can no longer be chosen. A row keeps its place while [query] and
/// [opening] stay the same (portable mention rules, section 6).
///
/// While the directory search can still add people for this query
/// ([pending]), `reserveHeight` holds until the query or the opening
/// changes, so a chooser can keep its height and a late row fills empty
/// space instead of moving the row under a finger.
///
/// With [restartForNewAgents], the shown rows start over when [ranked]
/// offers an agent this query and opening have not shown, so an agent that
/// loads after people filled the list still takes its place ahead of them.
/// An agent that drops out during a reload keeps its row, marked
/// unavailable, and its return moves nothing.
({List<MentionCandidate> rows, Set<String> unavailable, bool reserveHeight})
useStableMentionRows(
  List<MentionCandidate> ranked, {
  required String query,
  required int opening,
  required bool pending,
  bool restartForNewAgents = false,
}) {
  final shown =
      useRef<(String, int, List<MentionCandidate>, bool, Set<String>)?>(null);
  final last = shown.value;
  final kept = last != null && last.$1 == query && last.$2 == opening;
  final seenAgents = kept ? last.$5 : <String>{};
  final same =
      kept &&
      !(restartForNewAgents &&
          ranked
              .take(mentionSuggestionLimit)
              .any((c) => c.isAgent && !seenAgents.contains(c.pubkey)));
  final result = stableMentionRows(same ? last.$3 : const [], ranked);
  seenAgents.addAll([
    for (final row in result.rows)
      if (row.isAgent) row.pubkey,
  ]);
  final reserveHeight = pending || (same && last.$4);
  shown.value = (query, opening, result.rows, reserveHeight, seenAgents);
  return (
    rows: result.rows,
    unavailable: result.unavailable,
    reserveHeight: reserveHeight,
  );
}

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
({List<MentionCandidate> rows, Set<String> unavailable, bool reserveHeight})
useStableMentionRows(
  List<MentionCandidate> ranked, {
  required String query,
  required int opening,
  required bool pending,
}) {
  final shown = useRef<(String, int, List<MentionCandidate>, bool)?>(null);
  final last = shown.value;
  final same = last != null && last.$1 == query && last.$2 == opening;
  final result = stableMentionRows(same ? last.$3 : const [], ranked);
  final reserveHeight = pending || (same && last.$4);
  shown.value = (query, opening, result.rows, reserveHeight);
  return (
    rows: result.rows,
    unavailable: result.unavailable,
    reserveHeight: reserveHeight,
  );
}

part of '../compose_bar.dart';

Set<String> _agentMentionLabels({
  required Map<String, MentionCandidate> bindings,
}) => {
  for (final entry in bindings.entries)
    if (entry.value.isAgent) entry.key,
};

List<MentionCandidate> _resolveComposerMentions(
  String text,
  Map<String, MentionCandidate> selected,
  List<MentionCandidate> members,
) {
  final candidates = <String, List<MentionCandidate>>{
    for (final e in selected.entries) e.key.toLowerCase(): [e.value],
  };
  final selectedNames = candidates.keys.toSet();
  for (final member in members) {
    final label = member.label.toLowerCase();
    if (!selectedNames.contains(label)) (candidates[label] ??= []).add(member);
  }
  final winners = <String, MentionCandidate>{};
  for (final range in mentionOccurrences(text, candidates.keys)) {
    final identities = {
      for (final c in candidates[range.label]!) c.pubkey.toLowerCase(): c,
    };
    if (identities.length > 1) {
      throw FormatException(
        'The mention @${range.label} is ambiguous. Choose a recipient from the mention picker.',
      );
    }
    winners.addAll(identities);
  }
  return winners.values.toList();
}

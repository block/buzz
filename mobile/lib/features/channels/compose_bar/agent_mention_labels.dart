part of '../compose_bar.dart';

/// Selected labels the editor draws as chips, each flagged as an agent. A
/// binding restored from a saved draft carries no agent bit, so it draws as
/// a person chip until the send revalidates it. Chips and recipients share
/// [mentionOccurrences], so a chip on screen is a recipient at send and text
/// that is no longer a chip notifies nobody.
Map<String, bool> _mentionChipLabels({
  required Map<String, MentionCandidate> bindings,
}) => {
  for (final entry in bindings.entries)
    if (entry.key.isNotEmpty) entry.key: entry.value.isAgent,
};

String _mentionChipLabelsKey(Map<String, bool> labels) =>
    (labels.entries.map((e) => '${e.key}\u0001${e.value}').toList()..sort())
        .join('\u0000');

List<MentionCandidate> _resolveComposerMentions(
  String text,
  Map<String, MentionCandidate> selected,
  List<MentionCandidate> members,
  List<MentionCandidate> restoredCandidates,
) {
  // A broken record cannot fall back to a same-name roster entry.
  if (selected.values.any((c) => c.pubkey.isEmpty)) {
    throw const FormatException(
      'Saved mention identity is invalid. Clear the draft and select again.',
    );
  }
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
    for (final entry in identities.entries) {
      final selected = entry.value;
      final current = restoredCandidates
          .where((c) => c.pubkey == entry.key)
          .firstOrNull;
      if (selected.requiresRevalidation && current == null) {
        throw const FormatException(
          'Saved mention is no longer available. Select it again from the picker.',
        );
      }
      winners[entry.key] = selected.requiresRevalidation ? current! : selected;
    }
  }
  return winners.values.toList();
}

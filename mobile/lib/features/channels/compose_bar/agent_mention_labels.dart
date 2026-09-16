part of '../compose_bar.dart';

// Keep only untouched generated ranges. Editing a mention makes it authored
// text; editing/deleting another mention must not lose the remaining ranges.
class _AutomaticAgentMentions {
  Set<String> _labels = {};
  List<({int start, int end})> _ranges = [];

  void seed(String text, Iterable<String> labels) {
    _labels = labels.toSet();
    observe(text);
  }

  void observe(String text) {
    final occurrences = mentionOccurrences(text, _labels);
    // Exact labels carry identity (including same-name qualifiers). A deleted,
    // edited, or duplicated literal ceases to be automatically owned. Do not
    // reacquire it if the user subsequently types it again.
    _labels.removeWhere((label) {
      final matches = occurrences.where((range) => range.label == label);
      return matches.length != 1 ||
          text.substring(matches.single.start, matches.single.end) != '@$label';
    });
    _ranges = [
      for (final range in occurrences)
        if (_labels.contains(range.label))
          (
            start: range.start,
            end: range.end < text.length && text[range.end] == ' '
                ? range.end + 1
                : range.end,
          ),
    ];
  }

  TextEditingValue removeFrom(TextEditingValue value) {
    observe(value.text);
    var text = value.text;
    var offset = value.selection.baseOffset.clamp(0, text.length);
    for (final range in _ranges.reversed) {
      text = text.replaceRange(range.start, range.end, '');
      if (offset > range.start) {
        offset -= (offset - range.start).clamp(0, range.end - range.start);
      }
    }
    seed('', const []);
    return TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: offset),
    );
  }
}

void _useInitialAgentMentions({
  required TextEditingController controller,
  required ObjectRef<Map<String, MentionCandidate>> mentionMap,
  required ObjectRef<String> automaticPrefix,
  required _AutomaticAgentMentions automaticMentions,
  required ObjectRef<bool> isModifyingText,
  required String identity,
  required bool enabled,
  required List<String> pubkeys,
  required List<ChannelMember>? members,
  required Map<String, UserProfile> userCache,
  required Map<String, String> owners,
}) {
  final initialized = useRef(false);
  useEffect(() {
    initialized.value = false;
    return null;
  }, [identity, enabled]);
  useEffect(() {
    if (!enabled || initialized.value || members == null || pubkeys.isEmpty) {
      return null;
    }
    var current = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!current || initialized.value) return;
      initialized.value = true;
      // Never overwrite a restored draft, a deliberate deletion, or typing
      // that occurred while the roster was being fetched.
      if (controller.text.isNotEmpty) return;
      final recipients = pubkeys.map((key) => key.toLowerCase()).toSet();
      final candidates = buildMentionCandidates(
        members: members,
        relayAgents: const [],
        sharedChannelIds: const {},
        userCache: userCache,
        ownerByAgentPubkey: owners,
      );
      for (final candidate in candidates) {
        if (!candidate.isAgent || !recipients.contains(candidate.pubkey)) {
          continue;
        }
        final label = selectedMentionLabel(candidate.label, candidate.pubkey, {
          for (final entry in mentionMap.value.entries)
            entry.key: entry.value.pubkey,
        });
        mentionMap.value[label] = candidate;
      }
      final prefix = mentionMap.value.keys.map((label) => '@$label ').join();
      automaticPrefix.value = prefix;
      automaticMentions.seed(prefix, mentionMap.value.keys);
      isModifyingText.value = true;
      try {
        controller.value = TextEditingValue(
          text: prefix,
          selection: TextSelection.collapsed(offset: prefix.length),
        );
      } finally {
        isModifyingText.value = false;
      }
    });
    return () => current = false;
  }, [identity, enabled, members, pubkeys.join(','), userCache, owners]);
}

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

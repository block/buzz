part of '../compose_bar.dart';

/// A picked mention bound to its recipient at `[start, end)` of the draft,
/// the literal `@Label`. The position is the identity: two namesakes both
/// read `@Sam` in the text and keep their own keys. A binding restored from
/// a saved draft carries no agent bit, so it draws as a person chip until
/// the send revalidates it.
@immutable
class _ComposerMention {
  final int start;
  final int end;
  final MentionCandidate candidate;

  const _ComposerMention({
    required this.start,
    required this.end,
    required this.candidate,
  });

  /// The literal `@Label` saved with [draft] at `[start, end)` of [text]. A
  /// label qualified by an earlier Buzz (`Scout (<key>)`) displays as the
  /// plain name.
  factory _ComposerMention.restored(ComposeDraftMention draft, String text) =>
      _ComposerMention(
        start: draft.start,
        end: draft.end,
        candidate: MentionCandidate(
          pubkey: draft.pubkey,
          displayName: text
              .substring(draft.start + 1, draft.end)
              .replaceFirst(
                RegExp(r' \([0-9a-f]{64}\)$', caseSensitive: false),
                '',
              ),
          requiresRevalidation: true,
        ),
      );

  MentionRange get range => (start: start, end: end);

  String get pubkey => candidate.pubkey.toLowerCase();

  /// The name as written in [text], without the `@`.
  String label(String text) => text.substring(start + 1, end);

  _ComposerMention at(MentionRange range) => _ComposerMention(
    start: range.start,
    end: range.end,
    candidate: candidate,
  );

  ComposeDraftMention toDraft() =>
      ComposeDraftMention(start: start, end: end, pubkey: pubkey);
}

/// Display labels for chips whose name another candidate shares, by
/// lowercase key: the picked name with the chooser's short key suffix
/// (`Sam · a1b2`), from the same identity-name policy. The comparison
/// context is the channel's [members] and the picked [mentions] themselves,
/// so a single picked namesake is qualified, and so is a person picked from
/// the directory who shares a member's name. The key never enters the text.
Map<String, String> _mentionChipQualifiers(
  IdentityNameSources sources,
  List<MentionCandidate> members,
  List<_ComposerMention> mentions,
) {
  if (mentions.isEmpty) return const {};
  final names = mentionPickerNames(sources, [
    ...members,
    for (final mention in mentions) mention.candidate,
  ]);
  return {
    for (final mention in mentions)
      if (names.resolve(mention.pubkey)?.qualifier case final suffix?)
        mention.pubkey: '${mention.candidate.label} · $suffix',
  };
}

/// Keeps the editor's namesake chip labels current: a chip whose name
/// another candidate shares reads `Sam · a1b2`, the chooser's label, while
/// the draft text stays `@Sam`. [members] is read again whenever one of
/// [memberKeys] or the set of picked keys changes.
void _useMentionChipQualifiers({
  required WidgetRef ref,
  required _MarkdownEditingController controller,
  required List<_ComposerMention> mentions,
  required List<MentionCandidate> Function() members,
  required List<Object?> memberKeys,
}) {
  final identityNames = ref.watch(identityNameSourcesProvider);
  final mentionKeysKey = mentions.map((m) => m.pubkey).join(',');
  final chipQualifiers = useMemoized(
    () => _mentionChipQualifiers(identityNames, members(), mentions),
    [identityNames, ...memberKeys, mentionKeysKey],
  );
  useEffect(() {
    controller.setMentionQualifiers(chipQualifiers);
    return null;
  }, [controller, chipQualifiers]);
}

/// Whether [caret] sits inside, or at the end of, a mention bound at
/// [start]. Such an `@` does not open a query.
bool _insideSelectedMention(
  List<_ComposerMention> mentions,
  int start,
  int caret,
) => mentions.any((m) => m.start == start && caret <= m.end);

/// The recipients a send notifies, in text order: every bound mention, then
/// typed names outside a chip. A typed name a chip also carries resolves to
/// that chip's key; otherwise an exact, unique member name notifies, and a
/// name several identities share is refused so nobody is silently dropped.
List<MentionCandidate> _resolveComposerMentions(
  String text,
  List<_ComposerMention> mentions, {
  required bool mentionsInvalid,
  required List<MentionCandidate> members,
  required List<MentionCandidate> restoredCandidates,
}) {
  // A broken record cannot fall back to a same-name roster entry.
  if (mentionsInvalid || mentions.any((m) => m.candidate.pubkey.isEmpty)) {
    throw const FormatException(
      'Saved mention identity is invalid. Clear the draft and select again.',
    );
  }
  MentionCandidate resolve(MentionCandidate selected) {
    if (!selected.requiresRevalidation) return selected;
    final key = selected.pubkey.toLowerCase();
    final current = restoredCandidates
        .where((c) => c.pubkey.toLowerCase() == key)
        .firstOrNull;
    if (current == null) {
      throw const FormatException(
        'Saved mention is no longer available. Select it again from the picker.',
      );
    }
    return current;
  }

  final found = <(int, MentionCandidate)>[
    for (final mention in mentions) (mention.start, resolve(mention.candidate)),
  ];
  final byName = <String, Map<String, MentionCandidate>>{};
  for (final mention in mentions) {
    (byName[mention.label(text).toLowerCase()] ??= {})[mention.pubkey] =
        mention.candidate;
  }
  final chipNames = byName.keys.toSet();
  for (final member in members) {
    final name = member.label.toLowerCase();
    if (chipNames.contains(name)) continue;
    (byName[name] ??= {})[member.pubkey.toLowerCase()] = member;
  }
  for (final range in mentionOccurrences(text, byName.keys)) {
    if (mentions.any((m) => m.start == range.start)) continue;
    final identities = byName[range.label]!;
    if (identities.length > 1) {
      throw FormatException(
        'The mention @${range.label} is ambiguous. Choose a recipient from the mention picker.',
      );
    }
    found.add((range.start, resolve(identities.values.single)));
  }
  found.sort((a, b) => a.$1.compareTo(b.$1));
  final winners = <String, MentionCandidate>{};
  for (final (_, candidate) in found) {
    winners[candidate.pubkey.toLowerCase()] ??= candidate;
  }
  return winners.values.toList();
}

/// The content sent for [text]. When bound mentions of different keys share
/// a name, ignoring case, each is written `@Name (<key>)` so readers tell
/// them apart, as buzz-app's `composer-markdown.ts` does; one key under a
/// name stays bare, and a mention inside [code] the composer styles stays as
/// typed. The composer itself never showed the key.
String _qualifiedMentionContent(
  String text,
  List<_ComposerMention> mentions, {
  required List<TextRange> code,
}) {
  final keysByName = <String, Set<String>>{};
  for (final mention in mentions) {
    (keysByName[mention.label(text).toLowerCase()] ??= {}).add(mention.pubkey);
  }
  var content = text;
  for (final mention in mentions.reversed) {
    if (keysByName[mention.label(text).toLowerCase()]!.length < 2) continue;
    if (code.any((c) => c.start < mention.start && mention.end < c.end)) {
      continue;
    }
    content = content.replaceRange(
      mention.end,
      mention.end,
      ' (${mention.pubkey})',
    );
  }
  return content;
}

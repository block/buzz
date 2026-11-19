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

  /// Migrated from a label-keyed draft and not yet checked against the
  /// roster; see [_MarkdownEditingController.settleLegacyMentions].
  final bool legacy;

  const _ComposerMention({
    required this.start,
    required this.end,
    required this.candidate,
    this.legacy = false,
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
        legacy: draft.legacy,
      );

  MentionRange get range => (start: start, end: end);

  String get pubkey => candidate.pubkey.toLowerCase();

  /// The name as written in [text], without the `@`.
  String label(String text) => text.substring(start + 1, end);

  _ComposerMention at(MentionRange range) => _ComposerMention(
    start: range.start,
    end: range.end,
    candidate: candidate,
    legacy: legacy,
  );

  ComposeDraftMention toDraft() => ComposeDraftMention(
    start: start,
    end: end,
    pubkey: pubkey,
    legacy: legacy,
  );
}

/// The channel's members as a send reads them: by lowercase name, and
/// indexed for the text scan. Built once per roster, since both cost a pass
/// over every member; see [_useMemberMentions].
class _MemberMentions {
  _MemberMentions(this.candidates) {
    for (final member in candidates) {
      (byName[member.label.toLowerCase()] ??= {}).putIfAbsent(
        member.pubkey.toLowerCase(),
        () => member,
      );
    }
    labels = MentionLabelIndex(byName.keys);
  }

  final List<MentionCandidate> candidates;

  /// The members under each lowercase name, by lowercase key.
  final byName = <String, Map<String, MentionCandidate>>{};
  late final MentionLabelIndex labels;
}

/// The channel's members for the send path, read when asked for and kept
/// until one of [keys], the objects [build] reads, is replaced. A send, a
/// legacy draft and the chip labels then share one pass over the roster
/// however often they run.
_MemberMentions Function() _useMemberMentions(
  List<MentionCandidate> Function() build,
  List<Object?> keys,
) {
  final cache = useRef<(List<Object?>, _MemberMentions)?>(null);
  return () {
    final cached = cache.value;
    if (cached != null &&
        Iterable<int>.generate(
          keys.length,
        ).every((i) => identical(cached.$1[i], keys[i]))) {
      return cached.$2;
    }
    final members = _MemberMentions(build());
    cache.value = (keys, members);
    return members;
  };
}

extension _LegacyMentions on _MarkdownEditingController {
  /// Gives member names the occurrences the old send gave them. A draft
  /// saved with label-keyed bindings is read without the roster, so its
  /// bindings arrive marked legacy; here a longer name among [members]
  /// that covers one (`@Sam Smith` over a picked `@Sam`) takes it back,
  /// and the typed name resolves at send as it used to.
  /// Backticks read as spaces, as they did when the binding was saved.
  /// Once the roster is [complete] the surviving bindings are ordinary.
  void settleLegacyMentions(
    _MemberMentions Function() members, {
    required bool complete,
  }) {
    if (!_mentions.any((mention) => mention.legacy)) return;
    final occurrences = mentionOccurrences(text.replaceAll('`', ' '), {
      for (final mention in _mentions)
        if (mention.legacy) mention.label(text).toLowerCase(),
    }, known: members().labels);
    final settled = <_ComposerMention>[];
    for (final mention in _mentions) {
      if (!mention.legacy) {
        settled.add(mention);
      } else if (!occurrences.any(
        (range) => range.start <= mention.start && range.end > mention.end,
      )) {
        settled.add(
          complete
              ? _ComposerMention(
                  start: mention.start,
                  end: mention.end,
                  candidate: mention.candidate,
                )
              : mention,
        );
      }
    }
    if (!complete && settled.length == _mentions.length) return;
    replaceDraft(value, mentions: settled, mentionsInvalid: _mentionsInvalid);
  }
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
/// [memberKeys] or the set of picked keys changes. The picked keys are
/// read from the controller as it changes rather than from a rebuild: a
/// prefill that swaps one namesake agent for another changes them from
/// inside a build, where a rebuild request is dropped.
void _useMentionChipQualifiers({
  required WidgetRef ref,
  required _MarkdownEditingController controller,
  required List<MentionCandidate> Function() members,
  required List<Object?> memberKeys,
}) {
  final identityNames = ref.watch(identityNameSourcesProvider);
  useEffect(() {
    String? qualifiedFor;
    void update() {
      final keys = controller.mentions.map((m) => m.pubkey).join(',');
      if (keys == qualifiedFor) return;
      qualifiedFor = keys;
      controller.setMentionQualifiers(
        _mentionChipQualifiers(identityNames, members(), controller.mentions),
      );
    }

    update();
    controller.addListener(update);
    return () => controller.removeListener(update);
  }, [controller, identityNames, ...memberKeys]);
}

/// Whether a mention is bound at [start]. Its `@` opens no query, wherever
/// the caret is: the prose typed after a chip is never a name search. A
/// name character typed against the chip unbinds it first, so the `@` of
/// `@Bobby` is a query again.
bool _startsSelectedMention(List<_ComposerMention> mentions, int start) =>
    mentions.any((m) => m.start == start);

/// The recipients a send notifies, in text order: every bound mention, then
/// typed names outside a chip. A typed name notifies when it is exact and
/// unique among the chips and [members], ends as [typedMentionNotifies]
/// requires and is not a Markdown link's label, so `@Alice@example.org` and
/// a pasted `[@Alice](url)` notify nobody; a name several identities share is
/// refused so nobody is silently dropped. That holds beside a chip of the
/// same name too: text restored without its binding, as by a system Undo,
/// needs a pick rather than borrowing the surviving chip's key.
///
/// The text is scanned once, against the chips' names, the names their
/// recipients go by now and the [members] index. `nameEnds` is that scan:
/// where the longest known name starting at each `@` ends, which
/// [_qualifiedMentionContent] reads to see a chip that would draw as someone
/// else. A name known only as a chip recipient's current name notifies
/// nobody by being typed, and no shorter name inside it does either.
({List<MentionCandidate> recipients, Map<int, int> nameEnds})
_resolveComposerMentions(
  String text,
  List<_ComposerMention> mentions, {
  required bool mentionsInvalid,
  required _MemberMentions members,
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
  final nameEnds = <int, int>{};
  for (final range in mentionOccurrences(text, {
    ...byName.keys,
    for (final (_, candidate) in found) candidate.label.toLowerCase(),
  }, known: members.labels)) {
    nameEnds[range.start] = range.end;
    if (mentions.any((m) => m.start == range.start)) continue;
    if (!typedMentionNotifies(text, range.start, range.end)) continue;
    final identities = {
      ...?members.byName[range.label],
      ...?byName[range.label],
    };
    if (identities.isEmpty) continue;
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
  return (recipients: winners.values.toList(), nameEnds: nameEnds);
}

/// The content sent for [text]. A bound mention is written `@Name (<key>)`
/// when its plain text would read as someone else in the bubble: when bound
/// mentions of different keys share its name, ignoring case, as buzz-app's
/// `composer-markdown.ts` qualifies them, or when the text typed against it
/// spells a longer known name (`-bot` after a picked `@Bob`, beside a
/// `Bob-bot`), which the bubble's longest-name match would otherwise draw
/// as that other person. [nameEnds] says where the longest name at each `@`
/// ends, among the chips, the recipients and the channel's members; it is
/// the send's one scan, from [_resolveComposerMentions]. One key under a
/// name stays bare, and a mention inside [code] the composer styles stays
/// as typed. The composer itself never showed the key.
String _qualifiedMentionContent(
  String text,
  List<_ComposerMention> mentions, {
  required List<TextRange> code,
  required Map<int, int> nameEnds,
}) {
  final keysByName = <String, Set<String>>{};
  for (final mention in mentions) {
    (keysByName[mention.label(text).toLowerCase()] ??= {}).add(mention.pubkey);
  }
  var content = text;
  for (final mention in mentions.reversed) {
    if (keysByName[mention.label(text).toLowerCase()]!.length < 2 &&
        (nameEnds[mention.start] ?? 0) <= mention.end) {
      continue;
    }
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

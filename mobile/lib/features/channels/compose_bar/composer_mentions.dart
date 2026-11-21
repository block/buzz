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

/// The channel's members as a send reads them: by folded name
/// ([foldMentionLabel], as the text scan compares names), and
/// indexed for the text scan. Built once per roster, since both cost a pass
/// over every member; see [_useMemberMentions].
class _MemberMentions {
  _MemberMentions(this.candidates) {
    for (final member in candidates) {
      (byName[foldMentionLabel(member.label)] ??= {}).putIfAbsent(
        member.pubkey.toLowerCase(),
        () => member,
      );
    }
    labels = MentionLabelIndex(byName.keys);
  }

  final List<MentionCandidate> candidates;

  /// The members under each folded name, by lowercase key.
  final byName = <String, Map<String, MentionCandidate>>{};
  late final MentionLabelIndex labels;
}

/// The channel's members for the send path, read when asked for and kept
/// until one of [keys], the objects [build] reads, is replaced. A send and a
/// legacy draft then share one pass over the roster however often they run.
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
  /// The bindings [saved] restores. A label-keyed draft kept a label's key
  /// through backticks, and its send opened no mention directly after one.
  /// A migrated binding there that sits inside code or a link is dormant
  /// and stays, so the pick is back once the code goes; one after a
  /// backtick that closes no code (`` `@Bob hi ``) is dropped, so nobody is
  /// notified for it, as before the upgrade.
  List<_ComposerMention> restoredMentions(ComposeDraft? saved) {
    if (saved == null) return const [];
    final text = saved.text;
    final restored = [
      for (final mention in saved.mentions)
        _ComposerMention.restored(mention, text),
    ];
    bool afterBacktick(_ComposerMention mention) =>
        mention.legacy &&
        mention.start > 0 &&
        text.codeUnitAt(mention.start - 1) == 0x60;
    if (!restored.any(afterBacktick)) return restored;
    final live = _inProse(text, restored);
    return [
      for (final mention in restored)
        if (!afterBacktick(mention) || !live.contains(mention)) mention,
    ];
  }

  /// Gives member names the occurrences the old send gave them. A draft
  /// saved with label-keyed bindings is read without the roster, so its
  /// bindings arrive marked legacy; here a longer name among [members]
  /// that covers one (`@Sam Smith` over a picked `@Sam`) takes it back,
  /// and the typed name resolves at send as it used to.
  /// Once the roster is [complete] the surviving bindings are ordinary.
  void settleLegacyMentions(
    _MemberMentions Function() members, {
    required bool complete,
  }) {
    if (!_mentions.any((mention) => mention.legacy)) return;
    // Backticks read as spaces, as where the bindings were migrated: a
    // longer member name inside code claims its occurrence too.
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
    replaceDraft(
      value,
      mentions: settled,
      mentionsInvalid: _mentionsInvalid,
      sameDraft: true,
    );
  }
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
/// needs a pick rather than borrowing the surviving chip's key. A typed
/// name inside [code], the spans and blocks the composer styles as code, is
/// literal text and notifies nobody; a binding there is dormant and is not
/// among [mentions]. Nor does a typed name the bubble does not read as prose
/// ([mentionsInsideMarkdownLinks], the filter a chip passes): one inside a
/// bare URL such as `https://example.org/(@Bob)`, a link's label, destination
/// or title, an image, an autolink, a reference link, or code of any form
/// the bubble's parser reads, including double backticks and `~~~` fences.
/// Such a name is not refused as ambiguous either.
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
  required List<TextRange> code,
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
    (byName[foldMentionLabel(mention.label(text))] ??= {})[mention.pubkey] =
        mention.candidate;
  }
  final nameEnds = <int, int>{};
  final typed = <({int start, int end, String label})>[];
  for (final range in mentionOccurrences(text, {
    ...byName.keys,
    for (final (_, candidate) in found) candidate.label.toLowerCase(),
  }, known: members.labels)) {
    nameEnds[range.start] = range.end;
    if (mentions.any((m) => m.start == range.start)) continue;
    if (!typedMentionNotifies(text, range.start, range.end)) continue;
    if (code.any((c) => c.start < range.start && range.end < c.end)) continue;
    typed.add(range);
  }
  // One parse for the whole draft, and none without code or link syntax.
  final literal = mentionsInsideMarkdownLinks(text, [
    for (final range in typed) (start: range.start, end: range.end),
  ]);
  for (final (i, range) in typed.indexed) {
    if (literal.contains(i)) continue;
    final folded = foldMentionLabel(range.label);
    final identities = {...?members.byName[folded], ...?byName[folded]};
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
/// name stays bare. No live mention sits inside code, where the controller
/// keeps a binding dormant, so code is sent as typed. The composer itself
/// never showed the key.
String _qualifiedMentionContent(
  String text,
  List<_ComposerMention> mentions, {
  required Map<int, int> nameEnds,
}) {
  final keysByName = <String, Set<String>>{};
  for (final mention in mentions) {
    (keysByName[foldMentionLabel(mention.label(text))] ??= {}).add(
      mention.pubkey,
    );
  }
  var content = text;
  for (final mention in mentions.reversed) {
    if (keysByName[foldMentionLabel(mention.label(text))]!.length < 2 &&
        (nameEnds[mention.start] ?? 0) <= mention.end) {
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

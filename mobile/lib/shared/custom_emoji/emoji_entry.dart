import 'package:flutter/foundation.dart';

import '../relay/nostr_models.dart';
import 'custom_emoji.dart';

/// Most catalog emoji one search shows.
const int maxEmojiCatalogResults = 50;

/// Catalog emoji that survived validation, plus the names two authors both
/// published.
@immutable
class EmojiEntryResolution {
  /// One emoji per name, in the order the relay returned them.
  final List<CustomEmoji> emoji;

  /// Names published by more than one author. The newest event won each one.
  final List<String> conflicts;

  const EmojiEntryResolution({required this.emoji, required this.conflicts});
}

/// Read one kind:30180 event as a custom emoji, or null if it breaks the
/// contract: exactly one `d` tag that is a normalized shortcode, exactly one
/// `emoji` tag naming the same shortcode, and an http(s) URL.
CustomEmoji? emojiFromEntryEvent(NostrEvent event) {
  if (event.kind != EventKind.emojiEntry) return null;
  final dTags = [
    for (final tag in event.tags)
      if (tag.isNotEmpty && tag[0] == 'd') tag,
  ];
  final emojiTags = [
    for (final tag in event.tags)
      if (tag.isNotEmpty && tag[0] == 'emoji') tag,
  ];
  if (dTags.length != 1 || emojiTags.length != 1) return null;
  final d = dTags.single;
  final emoji = emojiTags.single;
  if (d.length < 2 || emoji.length < 3) return null;
  final name = normalizeShortcode(d[1]);
  if (name == null || name != d[1] || normalizeShortcode(emoji[1]) != name) {
    return null;
  }
  final uri = Uri.tryParse(emoji[2]);
  if (uri == null || !uri.hasAuthority) return null;
  if (uri.scheme != 'http' && uri.scheme != 'https') return null;
  return CustomEmoji(shortcode: name, url: emoji[2]);
}

bool _isNewer(NostrEvent a, NostrEvent b) =>
    a.createdAt > b.createdAt ||
    (a.createdAt == b.createdAt && a.id.compareTo(b.id) < 0);

/// Collapse kind:30180 [events] to one emoji per name.
///
/// The newest event wins for each author and name, then the newest author wins
/// for each name. Equal timestamps go to the smaller event id, so the result
/// does not depend on arrival order. Output is capped at
/// [maxEmojiCatalogResults].
EmojiEntryResolution resolveEmojiEntries(Iterable<NostrEvent> events) {
  final newestByAuthorAndName = <(String, String), NostrEvent>{};
  final emojiById = <String, CustomEmoji>{};
  final order = <String>[];
  for (final event in events) {
    final emoji = emojiFromEntryEvent(event);
    if (emoji == null) continue;
    emojiById[event.id] = emoji;
    final key = (event.pubkey, emoji.shortcode);
    final current = newestByAuthorAndName[key];
    if (current == null) {
      order.add(emoji.shortcode);
      newestByAuthorAndName[key] = event;
    } else if (_isNewer(event, current)) {
      newestByAuthorAndName[key] = event;
    }
  }

  final winnerByName = <String, NostrEvent>{};
  final authorsByName = <String, Set<String>>{};
  for (final entry in newestByAuthorAndName.entries) {
    final name = entry.key.$2;
    authorsByName.putIfAbsent(name, () => {}).add(entry.key.$1);
    final winner = winnerByName[name];
    if (winner == null || _isNewer(entry.value, winner)) {
      winnerByName[name] = entry.value;
    }
  }

  final seen = <String>{};
  final resolved = <CustomEmoji>[];
  for (final name in order) {
    if (!seen.add(name)) continue;
    resolved.add(emojiById[winnerByName[name]!.id]!);
  }
  return EmojiEntryResolution(
    emoji: resolved.take(maxEmojiCatalogResults).toList(),
    conflicts: [
      for (final entry in authorsByName.entries)
        if (entry.value.length > 1) entry.key,
    ],
  );
}

/// Turn what the user typed into catalog search text: no colons, lowercase,
/// `_` and `-` as spaces, like the content of a kind:30180 event. Returns null
/// below [minimum] characters.
String? emojiCatalogSearchText(String raw, {int minimum = 2}) {
  final text = raw
      .replaceAll(':', '')
      .replaceAll(RegExp(r'[_-]'), ' ')
      .trim()
      .toLowerCase()
      .replaceAll(RegExp(r'\s+'), ' ');
  return text.length < minimum ? null : text;
}

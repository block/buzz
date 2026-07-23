import 'package:flutter/foundation.dart';

import '../../shared/relay/nostr_models.dart';

/// A community custom emoji: a `:shortcode:` mapped to an image URL.
///
/// Mirrors desktop's NIP-30 model (kind:30030, per-user sets unioned into one
/// community palette). Identity downstream is shortcode-only — the palette must
/// never carry two URLs under one shortcode.
@immutable
class CustomEmoji {
  final String shortcode;
  final String url;

  const CustomEmoji({required this.shortcode, required this.url});

  @override
  bool operator ==(Object other) =>
      other is CustomEmoji && other.shortcode == shortcode && other.url == url;

  @override
  int get hashCode => Object.hash(shortcode, url);

  @override
  String toString() => 'CustomEmoji($shortcode -> $url)';
}

/// NIP-30 emoji set kind (parameterized-replaceable).
const int kindEmojiSet = 30030;

/// d-tag for a member's own custom emoji set.
const String customEmojiSetDTag = 'buzz:custom-emoji';

final RegExp _shortcodeRe = RegExp(r'^[a-z0-9_-]+$');

/// Normalize a shortcode the way the relay does: strip surrounding colons and
/// lowercase. Returns null if the result is empty or has invalid chars.
String? normalizeShortcode(String raw) {
  final stripped = raw
      .trim()
      .replaceFirst(RegExp(r'^:+'), '')
      .replaceFirst(RegExp(r':+$'), '');
  final lower = stripped.toLowerCase();
  return _shortcodeRe.hasMatch(lower) ? lower : null;
}

/// Parse NIP-30 `["emoji", shortcode, url]` tags from one event's tags into a
/// custom-emoji list. Shortcodes are normalized; malformed/duplicate entries
/// within the one event are skipped (first wins).
List<CustomEmoji> customEmojiFromTags(List<List<String>> tags) {
  final seen = <String>{};
  final emoji = <CustomEmoji>[];

  for (final tag in tags) {
    if (tag.isEmpty || tag[0] != 'emoji') continue;
    if (tag.length < 3) continue;
    final rawShortcode = tag[1];
    final url = tag[2];
    if (rawShortcode.isEmpty || url.isEmpty) continue;
    final shortcode = normalizeShortcode(rawShortcode);
    if (shortcode == null) continue;
    if (seen.contains(shortcode)) continue;
    seen.add(shortcode);
    emoji.add(CustomEmoji(shortcode: shortcode, url: url));
  }

  return emoji;
}

/// Union every member's kind:30030 set into the community palette, collapsed to
/// one entry per shortcode. When members disagree on a shortcode's URL, the
/// most recently published set wins (`created_at` is signed event data, so the
/// result stays deterministic and fetch-order-independent); equal timestamps
/// tie-break to the lexicographically-smallest URL. Output is sorted by
/// shortcode.
List<CustomEmoji> unionCustomEmoji(Iterable<NostrEvent> events) {
  final byShortcode = <String, ({String url, int createdAt})>{};
  for (final event in events) {
    for (final e in customEmojiFromTags(event.tags)) {
      final winner = byShortcode[e.shortcode];
      if (winner == null ||
          event.createdAt > winner.createdAt ||
          (event.createdAt == winner.createdAt &&
              e.url.compareTo(winner.url) < 0)) {
        byShortcode[e.shortcode] = (url: e.url, createdAt: event.createdAt);
      }
    }
  }
  final result =
      byShortcode.entries
          .map((e) => CustomEmoji(shortcode: e.key, url: e.value.url))
          .toList()
        ..sort((a, b) => a.shortcode.compareTo(b.shortcode));
  return result;
}

final RegExp _shortcodeScan = RegExp(r':([a-z0-9_-]+):', caseSensitive: false);

/// Return one `["emoji", shortcode, url]` tag per *distinct* known custom emoji
/// referenced in [content]. Matched case-insensitively; the canonical lowercase
/// shortcode is emitted. Unknown `:foo:` are ignored. Order follows first
/// appearance; each shortcode emitted at most once.
///
/// Mirrors how @mentions become `p` tags: derived from the final content at
/// send time so the event is self-contained for any NIP-30 client.
List<List<String>> buildCustomEmojiTags(
  String content,
  List<CustomEmoji> customEmoji,
) {
  if (customEmoji.isEmpty) return [];
  final urlByShortcode = {for (final e in customEmoji) e.shortcode: e.url};

  final emitted = <String>{};
  final tags = <List<String>>[];

  for (final match in _shortcodeScan.allMatches(content)) {
    final shortcode = match.group(1)!.toLowerCase();
    if (emitted.contains(shortcode)) continue;
    final url = urlByShortcode[shortcode];
    if (url == null) continue;
    emitted.add(shortcode);
    tags.add(['emoji', shortcode, url]);
  }

  return tags;
}

/// Resolve the image URL for a reaction whose content is a custom-emoji
/// `:shortcode:`, from the community palette. Returns null for unicode
/// reactions or unknown shortcodes.
String? reactionEmojiUrl(String emoji, List<CustomEmoji>? palette) {
  if (palette == null) return null;
  if (!emoji.startsWith(':') || !emoji.endsWith(':')) return null;
  final shortcode = emoji.substring(1, emoji.length - 1).toLowerCase();
  for (final e in palette) {
    if (e.shortcode == shortcode) return e.url;
  }
  return null;
}

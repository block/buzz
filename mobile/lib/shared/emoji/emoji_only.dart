/// Detection and sizing for messages whose entire body is emoji, ported from
/// desktop's `isEmojiOnlyMessage` (`desktop/src/shared/lib/emojiOnly.ts`) and
/// the `emojiOnly` branch of `MessageRow.tsx`.
library;

import 'package:characters/characters.dart';

import '../custom_emoji/custom_emoji.dart';

/// Body size for an emoji-only message. Desktop steps `text-sm` (14px) up to
/// `text-4xl`; mobile body text is the same 14px, so the same 36px lands the
/// same jump.
const double kEmojiOnlyFontSize = 36.0;

/// Line height for an emoji-only body — desktop's `leading-tight`. Emoji have
/// generous internal leading already, so normal line spacing leaves a gap that
/// reads as a blank row.
const double kEmojiOnlyHeight = 1.25;

/// Inline custom-emoji size inside an emoji-only message. Desktop sizes these
/// at `1.45em` of the surrounding text.
const double kEmojiOnlyCustomEmojiSize = kEmojiOnlyFontSize * 1.45;

/// Anything with `Extended_Pictographic` in it is emoji enough. Keycaps and
/// digit-based sequences don't match — those are covered by the dataset's own
/// native set instead, exactly as desktop does it.
final RegExp _pictographic = RegExp(
  r'\p{Extended_Pictographic}',
  unicode: true,
);

/// Whether [content] is nothing but emoji — native glyphs, known custom
/// `:shortcode:`s, and whitespace, with at least one emoji present.
///
/// [nativeEmoji] is the dataset's glyph set — read `nativeEmojiGlyphsProvider`.
/// Passing an empty set still gives correct results for ordinary emoji, since
/// the pictographic check stands on its own; only glyphs with no pictographic
/// code point of their own (keycaps like 1️⃣) need the set.
bool isEmojiOnlyMessage(
  String content, {
  required Set<String> nativeEmoji,
  List<CustomEmoji> customEmoji = const [],
}) {
  final trimmed = content.trim();
  if (trimmed.isEmpty) return false;

  final shortcodes = {
    for (final emoji in customEmoji) emoji.shortcode.toLowerCase(),
  };
  var sawEmoji = false;

  // Grapheme clusters, so a ZWJ family or a flag pair is one unit. Desktop
  // hand-rolls this scan; Dart's `characters` already segments correctly.
  final iterator = trimmed.characters.iterator;
  final clusters = <String>[];
  while (iterator.moveNext()) {
    clusters.add(iterator.current);
  }

  var index = 0;
  while (index < clusters.length) {
    final cluster = clusters[index];

    if (cluster.trim().isEmpty) {
      index += 1;
      continue;
    }

    if (cluster == ':') {
      final end = clusters.indexOf(':', index + 1);
      if (end <= index + 1) return false;
      final shortcode = clusters.sublist(index + 1, end).join().toLowerCase();
      if (!shortcodes.contains(shortcode)) return false;
      sawEmoji = true;
      index = end + 1;
      continue;
    }

    if (!nativeEmoji.contains(cluster) && !_pictographic.hasMatch(cluster)) {
      return false;
    }
    sawEmoji = true;
    index += 1;
  }

  return sawEmoji;
}

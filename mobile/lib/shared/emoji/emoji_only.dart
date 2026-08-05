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

/// Whether a scalar is in Unicode's `Extended_Pictographic` property.
///
/// Dart regular expressions do not support Unicode property escapes. Keep the
/// compact BMP ranges here and cover the contiguous supplementary emoji
/// blocks; keycaps deliberately remain dataset-only, matching desktop.
bool _isExtendedPictographic(int value) {
  if (value >= 0x1F000 && value <= 0x1FAFF) return true;

  const singletons = {
    0x00A9,
    0x00AE,
    0x203C,
    0x2049,
    0x2122,
    0x2139,
    0x2328,
    0x2388,
    0x23CF,
    0x24C2,
    0x25B6,
    0x25C0,
    0x260E,
    0x2611,
    0x2618,
    0x261D,
    0x2620,
    0x2626,
    0x262A,
    0x262E,
    0x262F,
    0x2638,
    0x2640,
    0x2642,
    0x2660,
    0x2663,
    0x2665,
    0x2666,
    0x2668,
    0x267B,
    0x2699,
    0x269B,
    0x269C,
    0x26A0,
    0x26A1,
    0x26A7,
    0x26C8,
    0x26D1,
    0x26D3,
    0x26D4,
    0x26E9,
    0x26EA,
    0x26F0,
    0x26F1,
    0x26F4,
    0x26F5,
    0x26FD,
    0x2702,
    0x2705,
    0x270F,
    0x2712,
    0x2714,
    0x2716,
    0x271D,
    0x2721,
    0x2728,
    0x2744,
    0x2747,
    0x274C,
    0x274E,
    0x2757,
    0x2763,
    0x2764,
    0x27A1,
    0x27B0,
    0x27BF,
    0x2B50,
    0x2B55,
    0x3030,
    0x303D,
    0x3297,
    0x3299,
  };
  if (singletons.contains(value)) return true;

  const ranges = [
    (0x2194, 0x2199),
    (0x21A9, 0x21AA),
    (0x231A, 0x231B),
    (0x23E9, 0x23F3),
    (0x23F8, 0x23FA),
    (0x25AA, 0x25AB),
    (0x25FB, 0x25FE),
    (0x2600, 0x2604),
    (0x2614, 0x2615),
    (0x2622, 0x2623),
    (0x2639, 0x263A),
    (0x2648, 0x2653),
    (0x265F, 0x2660),
    (0x267E, 0x267F),
    (0x2692, 0x2697),
    (0x26AA, 0x26AB),
    (0x26B0, 0x26B1),
    (0x26BD, 0x26BE),
    (0x26C4, 0x26C5),
    (0x26CE, 0x26CF),
    (0x26F2, 0x26F3),
    (0x26F7, 0x26FA),
    (0x2708, 0x270D),
    (0x2733, 0x2734),
    (0x2753, 0x2755),
    (0x2795, 0x2797),
    (0x2934, 0x2935),
    (0x2B05, 0x2B07),
    (0x2B1B, 0x2B1C),
  ];
  return ranges.any((range) => value >= range.$1 && value <= range.$2);
}

bool _containsExtendedPictographic(String cluster) =>
    cluster.runes.any(_isExtendedPictographic);

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

    if (!nativeEmoji.contains(cluster) &&
        !_containsExtendedPictographic(cluster)) {
      return false;
    }
    sawEmoji = true;
    index += 1;
  }

  return sawEmoji;
}

import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart' show rootBundle;
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'emoji_data.dart';

/// Asset path for the generated dataset. See `mobile/scripts/generate-emoji-data.mjs`.
const emojiDataAssetPath = 'assets/emoji/emoji-data.json';

/// Decode + parse off the UI isolate. ~1.9k entries is fast but not free, and
/// the picker sheet animates in while this runs.
EmojiDataset _parseEmojiData(String source) {
  return EmojiDataset.fromJson(jsonDecode(source) as Map<String, dynamic>);
}

/// The standard emoji dataset, loaded once from the app bundle.
final emojiDatasetProvider = FutureProvider<EmojiDataset>((ref) async {
  final source = await rootBundle.loadString(emojiDataAssetPath);
  return compute(_parseEmojiData, source);
});

/// Synchronous read of the dataset — [EmojiDataset.empty] while loading or on
/// error. Convenience for widgets that only need the resolved value, mirroring
/// `customEmojiListProvider`.
final emojiDatasetOrEmptyProvider = Provider<EmojiDataset>((ref) {
  return ref.watch(emojiDatasetProvider).value ?? EmojiDataset.empty;
});

/// Every glyph in the dataset, as a set for membership tests.
///
/// Built once per dataset load rather than per message: `isEmojiOnlyMessage`
/// runs on every rendered body, and materializing ~1.9k keys there would be a
/// per-frame cost.
final nativeEmojiGlyphsProvider = Provider<Set<String>>((ref) {
  return ref.watch(emojiDatasetOrEmptyProvider).nativeToShortcode.keys.toSet();
});

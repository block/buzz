import 'dart:convert';

/// Desktop-compatible colors offered for emoji avatar backgrounds.
const emojiAvatarColors = <int>[
  0xFFFFF4CC,
  0xFFFFE75C,
  0xFFFFB84D,
  0xFFFF8652,
  0xFFF6534F,
  0xFFFF6B9A,
  0xFFFB60C4,
  0xFFD66BFF,
  0xFFB141FF,
  0xFF7C5CFF,
  0xFF476CFF,
  0xFF3399FF,
  0xFF63C6F2,
  0xFF41EBC1,
  0xFF2ED3A2,
  0xFF73EF75,
  0xFF9FE870,
  0xFFC7D36F,
  0xFFCCCCCC,
  0xFF8A8F98,
  0xFF4B5563,
  0xFF000000,
  0xFFFFFFFF,
];

/// Creates the same inline SVG avatar URL used by the desktop editor.
String emojiAvatarDataUrl(String emoji, int colorValue) {
  final color =
      '#${(colorValue & 0xFFFFFF).toRadixString(16).padLeft(6, '0').toUpperCase()}';
  final escapedEmoji = emoji
      .replaceAll('&', '&amp;')
      .replaceAll('<', '&lt;')
      .replaceAll('>', '&gt;');
  final svg =
      '<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512">'
      '<rect width="512" height="512" rx="256" fill="$color"/>'
      '<text x="50%" y="56%" dominant-baseline="middle" text-anchor="middle" font-size="258">$escapedEmoji</text>'
      '</svg>';
  return 'data:image/svg+xml,${Uri.encodeComponent(svg)}';
}

/// The editable values encoded by an inline emoji avatar.
class EmojiAvatarData {
  /// Creates decoded emoji avatar values.
  const EmojiAvatarData({required this.emoji, required this.colorValue});

  /// The system emoji rendered in the avatar.
  final String emoji;

  /// The ARGB background color.
  final int colorValue;
}

/// Decodes an inline emoji-avatar data URL, or returns null for other images.
EmojiAvatarData? parseEmojiAvatarDataUrl(String? value) {
  final url = value?.trim();
  if (url == null || !url.startsWith('data:image/')) return null;
  try {
    final data = UriData.parse(url);
    if (data.mimeType != 'image/svg+xml') return null;
    return parseEmojiAvatarSvg(utf8.decode(data.contentAsBytes()));
  } on FormatException {
    return null;
  }
}

/// Decodes the editable values from an emoji-avatar SVG.
EmojiAvatarData? parseEmojiAvatarSvg(String svg) {
  final colorValue = RegExp(
    r'<rect\b[^>]*\sfill="([^"]+)"',
  ).firstMatch(svg)?[1];
  final emojiValue = RegExp(r'<text\b[^>]*>(.*?)</text>').firstMatch(svg)?[1];
  if (colorValue == null || emojiValue == null) return null;

  final hex = colorValue.startsWith('#') ? colorValue.substring(1) : colorValue;
  if (!RegExp(r'^[0-9a-fA-F]{6}$').hasMatch(hex)) return null;
  final rgb = int.tryParse(hex, radix: 16);
  if (rgb == null) return null;
  return EmojiAvatarData(
    emoji: emojiValue
        .replaceAll('&gt;', '>')
        .replaceAll('&lt;', '<')
        .replaceAll('&amp;', '&'),
    colorValue: 0xFF000000 | rgb,
  );
}

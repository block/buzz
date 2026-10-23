import 'package:flutter/material.dart';
import 'package:gpt_markdown/gpt_markdown.dart';

import '../relay/relay.dart';

import 'custom_emoji.dart';

/// Default rendered height of an inline custom emoji, in logical pixels.
/// Roughly matches a line of body text so emoji sit on the baseline cleanly.
const double kCustomEmojiInlineSize = 20.0;

/// A single custom-emoji image rendered as a square, network-loaded glyph.
///
/// Reused everywhere a `:shortcode:` resolves to an image: inline in message
/// bodies, reaction pills, status, and the picker. Falls back to the literal
/// `:shortcode:` text if the image fails to load, so a broken URL never leaves
/// a blank gap.
class CustomEmojiImage extends StatelessWidget {
  final String shortcode;
  final String url;
  final double size;

  const CustomEmojiImage({
    super.key,
    required this.shortcode,
    required this.url,
    this.size = kCustomEmojiInlineSize,
  });

  @override
  Widget build(BuildContext context) {
    final fallbackStyle = DefaultTextStyle.of(context).style;
    return MediaImage(
      url: url,
      width: size,
      height: size,
      decodeWidth: size,
      fit: BoxFit.contain,
      filterQuality: FilterQuality.medium,
      semanticLabel: ':$shortcode:',
      errorBuilder: (_, _, _) => Text(':$shortcode:', style: fallbackStyle),
    );
  }
}

/// gpt_markdown inline pattern that replaces `:shortcode:` with an inline
/// [CustomEmojiImage] for *known* shortcodes only. Unknown `:foo:` is left as
/// plain text. Matched case-insensitively; resolved via the lowercase palette.
///
/// Parallel to the mention and channel patterns in message_content.dart — add
/// an instance to `GptMarkdown.inlinePatterns` to enable custom emoji in any
/// markdown surface. The inherited default scopes keep it out of link labels:
/// its [WidgetSpan] nested inside a link's placeholder does not paint on iOS,
/// so an authored `[:emoji:](url)` would render as nothing.
class CustomEmojiPattern extends InlinePattern {
  /// Only include shortcodes present in the rendered [content]. gpt_markdown
  /// matches this pattern against every parsed text segment, so unrelated
  /// community emoji must not make every message expensive to parse.
  factory CustomEmojiPattern(
    List<CustomEmoji> palette, {
    required String content,
    double size = kCustomEmojiInlineSize,
  }) => CustomEmojiPattern._(_referencedUrls(palette, content), size);

  CustomEmojiPattern._(Map<String, String> urlByShortcode, double size)
    : super(
        pattern: _buildPattern(urlByShortcode.keys),
        builder: (context, match, style) {
          final raw = match.group(0)!;
          final shortcode = raw.substring(1, raw.length - 1).toLowerCase();
          final url = urlByShortcode[shortcode];
          if (url == null) return TextSpan(text: raw, style: style);
          return WidgetSpan(
            alignment: PlaceholderAlignment.middle,
            child: CustomEmojiImage(shortcode: shortcode, url: url, size: size),
          );
        },
      );

  // Look ahead so adjacent tokens sharing a colon are both considered:
  // :unknown:known: must still allow the known token to match.
  static final _shortcodeScan = RegExp(
    r'(?=:([a-z0-9_-]+):)',
    caseSensitive: false,
  );

  static Map<String, String> _referencedUrls(
    List<CustomEmoji> palette,
    String content,
  ) {
    final referenced = {
      for (final match in _shortcodeScan.allMatches(content))
        match.group(1)!.toLowerCase(),
    };
    if (referenced.isEmpty) return const {};
    return {
      for (final emoji in palette)
        if (referenced.contains(emoji.shortcode)) emoji.shortcode: emoji.url,
    };
  }

  /// Build a regex matching `:shortcode:` for any known shortcode, longest
  /// first so a longer name isn't shadowed by a shorter prefix. Matches nothing
  /// when the palette is empty (a regex that can never match).
  ///
  /// Not [InlinePattern.buildDelimitedPattern]: its word-boundary lookbehind
  /// would stop `:unknown:wave:` from rendering the known `:wave:`.
  static RegExp _buildPattern(Iterable<String> shortcodes) {
    final sorted = shortcodes.where((s) => s.trim().isNotEmpty).toSet().toList()
      ..sort((a, b) => b.length.compareTo(a.length));
    if (sorted.isEmpty) {
      // Never matches — gpt_markdown skips this pattern entirely.
      return RegExp(r'(?!x)x');
    }
    final alternatives = sorted.map(RegExp.escape).join('|');
    return RegExp(':(?:$alternatives):', caseSensitive: false);
  }
}

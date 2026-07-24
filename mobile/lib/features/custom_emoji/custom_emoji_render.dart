import 'package:flutter/material.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:gpt_markdown/custom_widgets/markdown_config.dart';

import '../../shared/relay/relay.dart';

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

/// gpt_markdown inline component that replaces `:shortcode:` with an inline
/// [CustomEmojiImage] for *known* shortcodes only. Unknown `:foo:` is left as
/// plain text. Matched case-insensitively; resolved via the lowercase palette.
///
/// Parallel to the `_MentionMd` / `_ChannelLinkMd` components in
/// message_content.dart — add an instance to a `GptMarkdown.inlineComponents`
/// list (before the default components) to enable custom emoji in any markdown
/// surface.
class CustomEmojiMd extends InlineMd {
  final Map<String, String> _urlByShortcode;
  final double size;
  late final RegExp _exp = _buildPattern(_urlByShortcode.keys);

  CustomEmojiMd(List<CustomEmoji> palette, {this.size = kCustomEmojiInlineSize})
    : _urlByShortcode = {for (final e in palette) e.shortcode: e.url};

  @override
  RegExp get exp => _exp;

  @override
  InlineSpan span(BuildContext context, String text, GptMarkdownConfig config) {
    final raw = exp.firstMatch(text.trim())?.group(0);
    if (raw == null) {
      return TextSpan(text: text, style: config.style);
    }
    final shortcode = raw.substring(1, raw.length - 1).toLowerCase();
    final url = _urlByShortcode[shortcode];
    if (url == null) {
      return TextSpan(text: text, style: config.style);
    }
    return WidgetSpan(
      alignment: PlaceholderAlignment.middle,
      child: CustomEmojiImage(shortcode: shortcode, url: url, size: size),
    );
  }

  /// Build a regex matching `:shortcode:` for any known shortcode, longest
  /// first so a longer name isn't shadowed by a shorter prefix. Matches nothing
  /// when the palette is empty (a regex that can never match).
  static RegExp _buildPattern(Iterable<String> shortcodes) {
    final sorted = shortcodes.where((s) => s.trim().isNotEmpty).toSet().toList()
      ..sort((a, b) => b.length.compareTo(a.length));
    if (sorted.isEmpty) {
      // Never matches — gpt_markdown skips this component entirely.
      return RegExp(r'(?!x)x');
    }
    final alternatives = sorted.map(RegExp.escape).join('|');
    return RegExp(':(?:$alternatives):', caseSensitive: false);
  }
}

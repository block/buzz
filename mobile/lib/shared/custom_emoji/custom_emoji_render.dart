import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../relay/relay.dart';
import '../theme/buzz_icons.dart';

import 'custom_emoji.dart';

/// Default rendered height of an inline custom emoji, in logical pixels.
/// Roughly matches a line of body text so emoji sit on the baseline cleanly.
const double kCustomEmojiInlineSize = 20.0;

/// Encoded bytes of custom emoji that [CustomEmojiImage] has downloaded, so a
/// native surface such as the iOS reactions sheet shows an emoji the timeline
/// already displays without fetching it again. Least recently used entries
/// are evicted past [maxTotalBytes]; an emoji over [maxEntryBytes] is not kept.
class CustomEmojiBytesCache {
  static const maxEntryBytes = 512 * 1024;
  static const maxTotalBytes = 8 * 1024 * 1024;

  final _entries = <String, Uint8List>{};
  var _totalBytes = 0;

  void remember(String url, Uint8List bytes) {
    if (bytes.lengthInBytes > maxEntryBytes) return;
    final previous = _entries.remove(url);
    if (previous != null) _totalBytes -= previous.lengthInBytes;
    _entries[url] = bytes;
    _totalBytes += bytes.lengthInBytes;
    while (_totalBytes > maxTotalBytes) {
      final oldest = _entries.keys.first;
      _totalBytes -= _entries.remove(oldest)!.lengthInBytes;
    }
  }

  Uint8List? lookup(String url) {
    final bytes = _entries.remove(url);
    if (bytes != null) _entries[url] = bytes;
    return bytes;
  }
}

/// Rebuilt with the media auth scope, like [MediaImageProvider]'s cache key,
/// so one account's downloads never reach another account's surfaces.
final customEmojiBytesCacheProvider = Provider<CustomEmojiBytesCache>((ref) {
  ref.watch(mediaGetAuthServiceProvider);
  return CustomEmojiBytesCache();
});

/// The neutral glyph that stands in for a custom emoji with no image: a muted
/// sparkles icon, because a truncated `:shortcode:` is illegible at glyph size.
/// The shortcode survives only as the semantics label.
class CustomEmojiPlaceholder extends StatelessWidget {
  final String shortcode;
  final double size;

  const CustomEmojiPlaceholder({
    super.key,
    required this.shortcode,
    required this.size,
  });

  @override
  Widget build(BuildContext context) => Icon(
    BuzzIcons.sparkles,
    size: size,
    color: DefaultTextStyle.of(context).style.color?.withValues(alpha: 0.6),
    semanticLabel: ':$shortcode:',
  );
}

/// A single custom-emoji image rendered as a square, network-loaded glyph.
///
/// Reused everywhere a `:shortcode:` resolves to an image: inline in message
/// bodies, reaction pills, status, and the picker. If the image fails to load,
/// it falls back to the literal `:shortcode:` text, so a broken URL never
/// leaves a blank gap in prose; a [compact] glyph, such as a reaction pill,
/// shows the [CustomEmojiPlaceholder] instead. A reaction slot is always
/// [compact]: it never renders shortcode text.
class CustomEmojiImage extends ConsumerWidget {
  final String shortcode;
  final String url;
  final double size;
  final bool compact;

  const CustomEmojiImage({
    super.key,
    required this.shortcode,
    required this.url,
    this.size = kCustomEmojiInlineSize,
    this.compact = false,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final fallbackStyle = DefaultTextStyle.of(context).style;
    final bytesCache = ref.watch(customEmojiBytesCacheProvider);
    return MediaImage(
      url: url,
      width: size,
      height: size,
      decodeWidth: size,
      fit: BoxFit.contain,
      filterQuality: FilterQuality.medium,
      semanticLabel: ':$shortcode:',
      onBytesLoaded: (bytes) => bytesCache.remember(url, bytes),
      errorBuilder: (_, _, _) => compact
          ? CustomEmojiPlaceholder(shortcode: shortcode, size: size)
          : Text(':$shortcode:', style: fallbackStyle),
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

  /// Only include shortcodes present in the rendered [content]. gpt_markdown
  /// embeds this pattern in a combined regex for every parsed text segment, so
  /// unrelated community emoji must not make every message expensive to parse.
  CustomEmojiMd(
    List<CustomEmoji> palette, {
    required String content,
    this.size = kCustomEmojiInlineSize,
  }) : _urlByShortcode = _referencedUrls(palette, content);

  /// Excluded from link labels: this component renders a [WidgetSpan], and a
  /// placeholder nested inside the link's own placeholder does not paint on
  /// iOS — an authored `[:emoji:](url)` renders as nothing. Link resolution
  /// wins over token detection inside a label.
  @override
  Set<MarkdownScope> get scopes => MarkdownComponent.allScopesExceptLinkLabel;

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

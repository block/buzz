import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:gpt_markdown/custom_widgets/markdown_config.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:url_launcher/url_launcher.dart';

import '../../shared/clipboard_utils.dart';
import '../../shared/syntax_highlight.dart';
import '../../shared/theme/theme.dart';
import '../custom_emoji/custom_emoji.dart';
import '../custom_emoji/custom_emoji_provider.dart';
import '../custom_emoji/custom_emoji_render.dart';
import 'media_viewer_page.dart';
import 'message_media.dart';

const _messageMediaMaxInlineWidth = 320.0;
const _messageMediaMaxImageHeight = 240.0;

/// Renders message content with markdown formatting, @mentions, #channel links,
/// and media-aware markdown images/videos.
class MessageContent extends HookConsumerWidget {
  final String content;

  /// Display names for mentioned pubkeys, extracted from event p-tags.
  /// Keys are lowercase pubkeys, values are display names.
  final Map<String, String> mentionNames;

  /// Known channel names for #channel links. Keys are lowercase channel
  /// names, values are channel IDs.
  final Map<String, String> channelNames;

  /// Raw event tags, used for `imeta` media metadata lookups.
  final List<List<String>> tags;

  /// Called when a #channel link is tapped.
  final void Function(String channelId)? onChannelTap;

  final TextStyle? baseStyle;

  final int? maxLines;

  const MessageContent({
    super.key,
    required this.content,
    this.mentionNames = const {},
    this.channelNames = const {},
    this.tags = const [],
    this.onChannelTap,
    this.baseStyle,
    this.maxLines,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final style =
        baseStyle ??
        context.textTheme.bodyMedium?.copyWith(color: context.colors.onSurface);
    final imetaByUrl = parseImetaTags(tags);
    final customEmoji = _mergeCustomEmoji(
      customEmojiFromTags(tags),
      ref.watch(customEmojiListProvider),
    );

    final finalContent = useMemoized(() {
      // Convert autolinks and bare URLs to standard markdown links,
      // but skip content inside backticks (inline code / fenced blocks).
      final buffer = StringBuffer();
      final parts = content.split('`');
      for (var i = 0; i < parts.length; i++) {
        if (i.isOdd) {
          // Inside backticks — preserve as-is.
          buffer.write('`${parts[i]}`');
        } else {
          // 1. Angle-bracket autolinks: <https://...>
          var segment = parts[i].replaceAllMapped(
            RegExp(r'<(https?://[^>]+)>'),
            (m) => '[${m[1]}](${m[1]})',
          );
          // 2. Bare URLs not already inside markdown link/image syntax.
          //    Negative lookbehind avoids matching URLs preceded by ]( or =
          //    which are already part of markdown links or imeta tags.
          segment = segment.replaceAllMapped(
            RegExp(r'(?<![(\]=])https?://[^\s)>\]]+'),
            (m) {
              final url = m[0]!;
              // Skip if this URL is already a markdown link label that equals
              // the URL (produced by step 1 or authored as [url](url)).
              final start = m.start;
              if (start >= 1 && segment[start - 1] == '[') return url;
              return '[$url]($url)';
            },
          );
          buffer.write(segment);
        }
      }
      final processed = buffer.toString();

      // Replace spaces with non-breaking spaces inside known mention names
      // so the gpt_markdown combined regex can match multi-word names
      // even when caseSensitive is not preserved.
      // Skip content inside backticks to avoid altering inline code.
      final mentionParts = processed.split('`');
      final mentionBuf = StringBuffer();
      for (var i = 0; i < mentionParts.length; i++) {
        if (i.isOdd) {
          mentionBuf.write('`${mentionParts[i]}`');
        } else {
          var segment = mentionParts[i];
          for (final name in mentionNames.values) {
            if (name.contains(' ')) {
              final normalizedName = _markdownMentionName(name);
              segment = segment.replaceAllMapped(
                RegExp('@${RegExp.escape(name)}', caseSensitive: false),
                (m) => '@$normalizedName',
              );
            }
          }
          mentionBuf.write(segment);
        }
      }
      final mentionProcessed = mentionBuf.toString();

      // Ensure channel links at the very start of content don't get
      // swallowed by markdown processing.
      var result = mentionProcessed;
      if (RegExp(r'^#[A-Za-z0-9_]').hasMatch(result)) {
        result = '\u200B$result';
      }
      return result;
    }, [content, mentionNames]);

    return GptMarkdown(
      finalContent,
      style: style,
      followLinkColor: false,
      codeBuilder: (context, name, code, closed) =>
          _MessageCodeBlock(name: name, code: code),
      linkBuilder: (context, linkText, url, linkStyle) =>
          _buildLink(context, linkText, url, linkStyle, style),
      imageBuilder: (context, imageUrl) =>
          _buildMedia(context, imageUrl, imetaByUrl[imageUrl]),
      maxLines: maxLines,
      inlineComponents: [
        _MentionMd(mentionNames: mentionNames),
        CustomEmojiMd(customEmoji),
        _ChannelLinkMd(channelNames: channelNames, onChannelTap: onChannelTap),
        ...MarkdownComponent.inlineComponents,
      ],
    );
  }

  Widget _buildMedia(BuildContext context, String imageUrl, ImetaEntry? imeta) {
    final mediaKind = classifyMediaUrl(imageUrl, imeta: imeta);
    if (mediaKind == MessageMediaKind.video) {
      return _MessageVideoPreview(url: imageUrl, imeta: imeta);
    }
    return _MessageImagePreview(
      url: imageUrl,
      imeta: imeta,
      semanticLabel: imeta?.alt ?? 'Message image',
    );
  }

  Widget _buildLink(
    BuildContext context,
    InlineSpan linkText,
    String url,
    TextStyle linkStyle,
    TextStyle? fallbackStyle,
  ) {
    String text = '';
    linkText.visitChildren((span) {
      if (span is TextSpan && span.text != null) {
        text += span.text!;
      }
      return true;
    });

    final baseStyle = fallbackStyle ?? linkStyle;

    return GestureDetector(
      onTap: () {
        final uri = Uri.tryParse(url);
        if (uri != null && (uri.scheme == 'http' || uri.scheme == 'https')) {
          launchUrl(uri, mode: LaunchMode.externalApplication);
        }
      },
      child: Text(
        text,
        style: baseStyle.copyWith(
          color: context.colors.primary,
          decoration: TextDecoration.underline,
          decorationColor: context.colors.primary,
        ),
      ),
    );
  }
}

List<CustomEmoji> _mergeCustomEmoji(
  List<CustomEmoji> eventEmoji,
  List<CustomEmoji> paletteEmoji,
) {
  if (eventEmoji.isEmpty) return paletteEmoji;
  if (paletteEmoji.isEmpty) return eventEmoji;
  final seen = <String>{};
  final merged = <CustomEmoji>[];
  for (final emoji in [...eventEmoji, ...paletteEmoji]) {
    if (seen.add(emoji.shortcode)) merged.add(emoji);
  }
  return merged;
}

class _MessageImagePreview extends HookWidget {
  final String url;
  final ImetaEntry? imeta;
  final String semanticLabel;

  const _MessageImagePreview({
    required this.url,
    required this.imeta,
    required this.semanticLabel,
  });

  @override
  Widget build(BuildContext context) {
    final heroTag = useMemoized(() => Object());
    final layout = _resolveImagePreviewLayout(context, imeta?.aspectRatio);

    return Padding(
      padding: const EdgeInsets.only(top: Grid.half),
      child: GestureDetector(
        onTap: () => openImageViewer(
          context,
          imageUrl: url,
          heroTag: heroTag,
          semanticLabel: semanticLabel,
        ),
        child: _MessageMediaPreviewFrame(
          previewKey: ValueKey('message-media-image-preview:$url'),
          backgroundColor: context.colors.surfaceContainerHighest,
          width: layout.width,
          height: layout.height,
          constraints: layout.constraints,
          child: Hero(
            tag: heroTag,
            child: Image.network(
              url,
              fit: layout.fit,
              semanticLabel: semanticLabel,
              errorBuilder: (_, _, _) => _MediaPreviewFallback(
                icon: LucideIcons.imageOff,
                label: 'Image unavailable',
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _MessageVideoPreview extends StatelessWidget {
  final String url;
  final ImetaEntry? imeta;

  const _MessageVideoPreview({required this.url, required this.imeta});

  @override
  Widget build(BuildContext context) {
    final rawAspectRatio = imeta?.aspectRatio ?? (16 / 9);
    final aspectRatio = rawAspectRatio.clamp(0.75, 1.91);
    final posterUrl = imeta?.posterUrl;

    return Padding(
      padding: const EdgeInsets.only(top: Grid.half),
      child: GestureDetector(
        onTap: () =>
            openVideoViewer(context, videoUrl: url, posterUrl: posterUrl),
        child: _MessageMediaPreviewFrame(
          previewKey: ValueKey('message-media-video-preview:$url'),
          backgroundColor: Colors.black,
          child: AspectRatio(
            aspectRatio: aspectRatio.toDouble(),
            child: Stack(
              fit: StackFit.expand,
              children: [
                if (posterUrl != null)
                  Image.network(
                    posterUrl,
                    fit: BoxFit.cover,
                    errorBuilder: (_, _, _) => const _MediaPreviewFallback(
                      icon: LucideIcons.video,
                      label: 'Video preview unavailable',
                    ),
                  )
                else
                  const _MediaPreviewFallback(
                    icon: LucideIcons.video,
                    label: 'Video attachment',
                  ),
                const ColoredBox(color: Color.fromRGBO(0, 0, 0, 0.28)),
                Center(
                  child: Container(
                    width: 52,
                    height: 52,
                    decoration: const BoxDecoration(
                      color: Color.fromRGBO(0, 0, 0, 0.6),
                      shape: BoxShape.circle,
                    ),
                    child: const Icon(
                      LucideIcons.play,
                      color: Colors.white,
                      size: 24,
                    ),
                  ),
                ),
                Positioned(
                  left: Grid.xxs,
                  right: Grid.xxs,
                  bottom: Grid.xxs,
                  child: Text(
                    'Video',
                    style: context.textTheme.labelSmall?.copyWith(
                      color: Colors.white,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _MessageMediaPreviewFrame extends StatelessWidget {
  final Key previewKey;
  final Color backgroundColor;
  final double? width;
  final double? height;
  final BoxConstraints? constraints;
  final Widget child;

  const _MessageMediaPreviewFrame({
    required this.previewKey,
    required this.backgroundColor,
    this.width,
    this.height,
    this.constraints,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    final resolvedWidth = constraints == null
        ? (width ?? _messageMediaMaxWidth(context))
        : width;

    return Container(
      key: previewKey,
      width: resolvedWidth,
      height: height,
      constraints: constraints,
      clipBehavior: Clip.antiAlias,
      decoration: BoxDecoration(
        color: backgroundColor,
        borderRadius: BorderRadius.circular(Radii.md),
        border: Border.all(color: context.colors.outlineVariant),
      ),
      child: child,
    );
  }
}

double _messageMediaMaxWidth(BuildContext context) {
  return math
      .min(MediaQuery.sizeOf(context).width * 0.72, _messageMediaMaxInlineWidth)
      .toDouble();
}

_ImagePreviewLayout _resolveImagePreviewLayout(
  BuildContext context,
  double? aspectRatio,
) {
  if (aspectRatio == null) {
    return _ImagePreviewLayout(
      constraints: BoxConstraints(
        maxWidth: _messageMediaMaxWidth(context),
        maxHeight: _messageMediaMaxImageHeight,
      ),
      fit: BoxFit.contain,
    );
  }

  final previewSize = _imagePreviewSize(context, aspectRatio);
  return _ImagePreviewLayout(
    width: previewSize.width,
    height: previewSize.height,
    fit: BoxFit.cover,
  );
}

Size _imagePreviewSize(BuildContext context, double? aspectRatio) {
  final maxWidth = _messageMediaMaxWidth(context);
  final safeAspectRatio = (aspectRatio ?? 1.0).clamp(0.2, 4.0).toDouble();

  var width = maxWidth;
  var height = width / safeAspectRatio;
  if (height > _messageMediaMaxImageHeight) {
    height = _messageMediaMaxImageHeight;
    width = height * safeAspectRatio;
  }

  return Size(width, height);
}

class _ImagePreviewLayout {
  final double? width;
  final double? height;
  final BoxConstraints? constraints;
  final BoxFit fit;

  const _ImagePreviewLayout({
    this.width,
    this.height,
    this.constraints,
    required this.fit,
  });
}

class _MediaPreviewFallback extends StatelessWidget {
  final IconData icon;
  final String label;

  const _MediaPreviewFallback({required this.icon, required this.label});

  @override
  Widget build(BuildContext context) {
    return ColoredBox(
      color: context.colors.surfaceContainerHighest,
      child: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, color: context.colors.onSurfaceVariant),
            const SizedBox(height: Grid.quarter),
            Text(
              label,
              style: context.textTheme.labelSmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
              textAlign: TextAlign.center,
            ),
          ],
        ),
      ),
    );
  }
}

class _MessageCodeBlock extends HookWidget {
  final String name;
  final String code;

  const _MessageCodeBlock({required this.name, required this.code});

  @override
  Widget build(BuildContext context) {
    final isCopied = useState(false);

    Future<void> handleCopy() async {
      await copyToClipboard(context, code, message: 'Copied code to clipboard');
      if (!context.mounted) return;
      isCopied.value = true;
      Future.delayed(const Duration(seconds: 2), () {
        if (context.mounted) isCopied.value = false;
      });
    }

    final codeBaseStyle = TextStyle(
      fontFamily: 'GeistMono',
      fontSize: 13,
      height: 1.5,
      color: context.colors.onSurface,
    );
    final isDark = context.theme.brightness == Brightness.dark;
    final codeTheme = isDark ? highlightDarkTheme : highlightLightTheme;
    final codeSpans = useMemoized(
      () => highlightCode(code, name, codeTheme, codeBaseStyle),
      [code, name, isDark],
    );
    return Container(
      margin: const EdgeInsets.only(top: Grid.half),
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHighest.withValues(alpha: 0.6),
        borderRadius: BorderRadius.circular(12),
        border: Border.all(
          color: context.colors.outline.withValues(alpha: 0.7),
        ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          if (name.isNotEmpty)
            Padding(
              padding: const EdgeInsets.only(
                left: Grid.twelve,
                top: Grid.half + Grid.quarter,
              ),
              child: Text(
                name,
                style: context.textTheme.labelSmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ),
          Stack(
            children: [
              Padding(
                padding: EdgeInsets.fromLTRB(
                  Grid.twelve,
                  name.isEmpty ? Grid.half + Grid.quarter : Grid.quarter,
                  44,
                  Grid.half + Grid.quarter,
                ),
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: RichText(
                    softWrap: false,
                    text: TextSpan(style: codeBaseStyle, children: codeSpans),
                  ),
                ),
              ),
              Positioned(
                top: 0,
                right: Grid.quarter,
                child: SizedBox(
                  width: 28,
                  height: 28,
                  child: IconButton(
                    onPressed: handleCopy,
                    padding: EdgeInsets.zero,
                    visualDensity: VisualDensity.compact,
                    style: IconButton.styleFrom(
                      tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                    ),
                    icon: Icon(
                      isCopied.value ? LucideIcons.check : LucideIcons.copy,
                      size: 14,
                      color: isCopied.value
                          ? context.colors.primary
                          : context.colors.onSurfaceVariant,
                    ),
                  ),
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

class _MentionMd extends InlineMd {
  final Map<String, String> mentionNames;
  late final RegExp _exp = _buildPrefixPattern(
    prefix: '@',
    knownNames: _mentionAliases(mentionNames.values),
    genericTokenPattern: r'[A-Za-z0-9_][A-Za-z0-9_\u00A0-]*',
  );

  _MentionMd({required this.mentionNames});

  @override
  RegExp get exp => _exp;

  @override
  InlineSpan span(
    BuildContext context,
    String text,
    final GptMarkdownConfig config,
  ) {
    final raw = exp.firstMatch(text.trim())?.group(0);
    if (raw == null) {
      return TextSpan(text: text, style: config.style);
    }

    final name = raw.substring(1).replaceAll('\u00A0', ' ').toLowerCase();
    String? displayName;
    for (final entry in mentionNames.entries) {
      final entryName = entry.value.toLowerCase();
      final firstName = entryName.split(RegExp(r'\s+')).first;
      if (entryName == name || firstName == name) {
        displayName = entry.value;
        break;
      }
    }

    return WidgetSpan(
      alignment: PlaceholderAlignment.baseline,
      baseline: TextBaseline.alphabetic,
      child: _TokenPill(
        text: '@${displayName ?? raw.substring(1)}',
        textStyle: config.style,
      ),
    );
  }
}

class _ChannelLinkMd extends InlineMd {
  final Map<String, String> channelNames;
  final void Function(String channelId)? onChannelTap;
  late final RegExp _exp = _buildPrefixPattern(
    prefix: '#',
    knownNames: channelNames.keys,
    genericTokenPattern: r'[A-Za-z0-9_][A-Za-z0-9_-]*',
  );

  _ChannelLinkMd({required this.channelNames, this.onChannelTap});

  @override
  RegExp get exp => _exp;

  @override
  InlineSpan span(
    BuildContext context,
    String text,
    final GptMarkdownConfig config,
  ) {
    final raw = exp.firstMatch(text.trim())?.group(0);
    if (raw == null) {
      return TextSpan(text: text, style: config.style);
    }

    final channelId = channelNames[raw.substring(1).toLowerCase()];
    final child = _TokenPill(
      text: raw,
      textStyle: config.style?.copyWith(fontWeight: FontWeight.w500),
    );

    return WidgetSpan(
      alignment: PlaceholderAlignment.baseline,
      baseline: TextBaseline.alphabetic,
      child: channelId != null && onChannelTap != null
          ? GestureDetector(onTap: () => onChannelTap!(channelId), child: child)
          : child,
    );
  }
}

class _TokenPill extends StatelessWidget {
  final String text;
  final TextStyle? textStyle;

  const _TokenPill({required this.text, this.textStyle});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
      decoration: BoxDecoration(
        color: context.colors.primary.withValues(alpha: 0.15),
        borderRadius: BorderRadius.circular(Radii.sm),
      ),
      child: Text(
        text,
        style:
            textStyle?.copyWith(color: context.colors.primary) ??
            context.textTheme.bodyMedium?.copyWith(
              color: context.colors.primary,
            ),
      ),
    );
  }
}

RegExp _buildPrefixPattern({
  required String prefix,
  required Iterable<String> knownNames,
  required String genericTokenPattern,
}) {
  final names =
      knownNames
          .map((name) => name.trim())
          .where((name) => name.isNotEmpty)
          .toSet()
          .toList()
        ..sort((a, b) => b.length.compareTo(a.length));

  final escapedPrefix = RegExp.escape(prefix);
  const leadingBoundary = r'(?<![\w./:-])';
  const trailingBoundary = r'(?=$|[\s,;.!?:)\]}])';

  if (names.isEmpty) {
    return RegExp(
      '$leadingBoundary$escapedPrefix(?:$genericTokenPattern)$trailingBoundary',
      caseSensitive: false,
      multiLine: true,
    );
  }

  final knownAlternatives = names.map(RegExp.escape).join('|');
  return RegExp(
    '$leadingBoundary$escapedPrefix(?:(?:$knownAlternatives)$trailingBoundary|(?:$genericTokenPattern)$trailingBoundary)',
    caseSensitive: false,
    multiLine: true,
  );
}

String _markdownMentionName(String name) => name.replaceAll(' ', '\u00A0');

Iterable<String> _mentionAliases(Iterable<String> mentionNames) sync* {
  for (final name in mentionNames) {
    final trimmed = name.trim();
    if (trimmed.isEmpty) continue;
    yield _markdownMentionName(trimmed);
    final firstName = trimmed.split(RegExp(r'\s+')).first;
    if (firstName.isNotEmpty) {
      yield firstName;
    }
  }
}

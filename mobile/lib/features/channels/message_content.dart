import 'dart:async';
import 'dart:io';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:gpt_markdown/custom_widgets/markdown_config.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:open_filex/open_filex.dart';
import 'package:path_provider/path_provider.dart';
import 'package:url_launcher/url_launcher.dart';

import '../../shared/clipboard_utils.dart';
import '../../shared/relay/relay.dart';
import '../../shared/syntax_highlight.dart';
import '../../shared/theme/theme.dart';
import '../../shared/custom_emoji/custom_emoji.dart';
import '../../shared/custom_emoji/custom_emoji_provider.dart';
import '../../shared/custom_emoji/custom_emoji_render.dart';
import '../../shared/emoji/emoji_data_provider.dart';
import '../../shared/emoji/emoji_only.dart';
import 'media_viewer_page.dart';
import 'message_media.dart';

part 'message_content/media_carousel.dart';

const _messageMediaMaxInlineWidth = 320.0;
const _messageMediaMaxImageHeight = 240.0;

typedef OpenDownloadedFile =
    Future<void> Function(
      String url,
      Map<String, String> headers,
      String filename,
    );

final openDownloadedFileProvider = Provider<OpenDownloadedFile>((ref) {
  final client = ref.watch(mediaHttpClientProvider);
  return (url, headers, filename) async {
    final response = await client.get(Uri.parse(url), headers: headers);
    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw HttpException(
        'Attachment download failed (${response.statusCode})',
        uri: Uri.parse(url),
      );
    }

    final directory = await getTemporaryDirectory();
    final safeName = _safeDownloadedFilename(filename);
    final file = File(
      '${directory.path}${Platform.pathSeparator}'
      '${DateTime.now().microsecondsSinceEpoch}-$safeName',
    );
    await file.writeAsBytes(response.bodyBytes, flush: true);
    final result = await OpenFilex.open(file.path);
    if (result.type != ResultType.done) {
      throw FileSystemException(result.message, file.path);
    }
  };
});

String _safeDownloadedFilename(String filename) {
  final safe = filename
      .split(RegExp(r'[/\\]'))
      .last
      .replaceAll(RegExp(r'[\x00-\x1F\x7F]'), '')
      .trim();
  return safe.isEmpty ? 'attachment' : safe;
}

/// Renders message content with markdown formatting, @mentions, #channel links,
/// and media-aware markdown images/videos.
class MessageContent extends HookConsumerWidget {
  final String content;

  /// Display names for mentioned pubkeys, extracted from event p-tags.
  /// Keys are lowercase pubkeys, values are display names.
  final Map<String, String> mentionNames;

  /// Mentioned pubkeys that resolve to agents. Agent chips use the desktop
  /// robot treatment instead of an `@` prefix.
  final Set<String> agentMentionPubkeys;

  /// Known channel names for #channel links. Keys are lowercase channel
  /// names, values are channel IDs.
  final Map<String, String> channelNames;

  /// Raw event tags, used for `imeta` media metadata lookups.
  final List<List<String>> tags;

  /// Called when a #channel link is tapped.
  final void Function(String channelId)? onChannelTap;

  /// Called when an @mention of a known user is tapped, with the
  /// mentioned user's pubkey.
  final void Function(String pubkey)? onMentionTap;

  /// Opens the message's thread from the full-screen image viewer.
  final VoidCallback? onMediaReply;

  /// Opens message-specific actions for an image in the full-screen viewer.
  final MediaViewerMoreAction? onMediaMore;

  final TextStyle? baseStyle;

  final int? maxLines;

  /// Render a body that is nothing but emoji at [kEmojiOnlyFontSize], the way
  /// desktop's `MessageRow` does. Off by default: previews, search hits, and
  /// notification rows want a message to occupy its usual line height whatever
  /// it contains.
  final bool scaleEmojiOnly;

  /// Allows a multi-image carousel to reclaim leading space reserved by the
  /// surrounding message layout, while keeping its image count aligned with
  /// the message body.
  final double mediaCarouselLeadingOverflow;

  /// Allows a multi-image carousel to continue through the trailing page
  /// gutter while keeping its first image and count aligned with the body.
  final double mediaCarouselTrailingOverflow;

  const MessageContent({
    super.key,
    required this.content,
    this.mentionNames = const {},
    this.agentMentionPubkeys = const {},
    this.channelNames = const {},
    this.tags = const [],
    this.onChannelTap,
    this.onMentionTap,
    this.onMediaReply,
    this.onMediaMore,
    this.baseStyle,
    this.maxLines,
    this.scaleEmojiOnly = false,
    this.mediaCarouselLeadingOverflow = 0,
    this.mediaCarouselTrailingOverflow = 0,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final baseTextStyle =
        baseStyle ??
        context.textTheme.bodyMedium?.copyWith(color: context.colors.onSurface);
    final resolvedMentionNames = mentionNames;
    final resolvedAgentMentionPubkeys = {
      ...agentMentionPubkeys.map((pubkey) => pubkey.toLowerCase()),
    };
    final imetaByUrl = parseImetaTags(tags);
    final trailingGallery = maxLines == null
        ? _extractTrailingImageGallery(content, imetaByUrl)
        : null;
    final markdownContent = trailingGallery?.content ?? content;
    final customEmoji = _mergeCustomEmoji(
      customEmojiFromTags(tags),
      ref.watch(customEmojiListProvider),
    );
    final mentionPresentationKey = [
      for (final entry
          in (resolvedMentionNames.entries.toList()
            ..sort((a, b) => a.key.compareTo(b.key))))
        '${entry.key}\u0000${entry.value}',
      ...(resolvedAgentMentionPubkeys.toList()..sort()),
    ].join('\u0001');

    // Decided here rather than by the caller: this is where the event's own
    // emoji tags and the community palette have already been merged, and a
    // `:shortcode:` only counts as emoji if it resolves against that palette.
    final emojiOnly =
        scaleEmojiOnly &&
        isEmojiOnlyMessage(
          markdownContent,
          nativeEmoji: ref.watch(nativeEmojiGlyphsProvider),
          customEmoji: customEmoji,
        );
    final style = emojiOnly
        ? baseTextStyle?.copyWith(
            fontSize: kEmojiOnlyFontSize,
            height: kEmojiOnlyHeight,
          )
        : baseTextStyle;
    final inlineCustomEmojiSize = emojiOnly
        ? kEmojiOnlyCustomEmojiSize
        : kCustomEmojiInlineSize;

    final finalContent = useMemoized(() {
      // Convert autolinks and bare URLs to standard markdown links,
      // but skip content inside backticks (inline code / fenced blocks).
      final buffer = StringBuffer();
      final parts = markdownContent.split('`');
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
          for (final name in resolvedMentionNames.values) {
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
    }, [markdownContent, resolvedMentionNames]);

    final markdown = KeyedSubtree(
      key: ValueKey('$finalContent\u0000$mentionPresentationKey'),
      child: GptMarkdown(
        finalContent,
        style: style,
        followLinkColor: false,
        codeBuilder: (context, name, code, closed) =>
            _MessageCodeBlock(name: name, code: code),
        linkBuilder: (context, linkText, url, linkStyle) =>
            _buildLink(context, ref, linkText, url, linkStyle, style),
        imageBuilder: (context, imageUrl) =>
            _buildMedia(context, imageUrl, imetaByUrl[imageUrl]),
        maxLines: maxLines,
        inlineComponents: [
          _MentionMd(
            mentionNames: resolvedMentionNames,
            agentMentionPubkeys: resolvedAgentMentionPubkeys,
            onMentionTap: onMentionTap,
          ),
          CustomEmojiMd(customEmoji, size: inlineCustomEmojiSize),
          _ChannelLinkMd(
            channelNames: channelNames,
            onChannelTap: onChannelTap,
          ),
          ...MarkdownComponent.inlineComponents,
        ],
      ),
    );
    if (trailingGallery == null) return markdown;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        if (trailingGallery.content.trim().isNotEmpty) markdown,
        _MessageImageCarousel(
          key: ValueKey(
            trailingGallery.items.map((item) => item.url).join('\u0000'),
          ),
          items: trailingGallery.items,
          leadingOverflow: mediaCarouselLeadingOverflow,
          trailingOverflow: mediaCarouselTrailingOverflow,
          onReply: onMediaReply,
          onMore: onMediaMore,
        ),
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
      onReply: onMediaReply,
      onMore: onMediaMore,
    );
  }

  Widget _buildLink(
    BuildContext context,
    WidgetRef ref,
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
      onTap: () async {
        final uri = Uri.tryParse(url);
        if (uri == null || (uri.scheme != 'http' && uri.scheme != 'https')) {
          return;
        }

        final auth = ref.read(mediaGetAuthServiceProvider);
        if (!auth.isRelayMediaUrl(url)) {
          await launchUrl(uri, mode: LaunchMode.externalApplication);
          return;
        }

        try {
          await ref.read(openDownloadedFileProvider)(
            url,
            auth.headersFor(url),
            text,
          );
        } catch (_) {
          if (!context.mounted) return;
          ScaffoldMessenger.maybeOf(context)?.showSnackBar(
            const SnackBar(content: Text('Could not open attachment')),
          );
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

class _MessageImagePreview extends HookConsumerWidget {
  final String url;
  final ImetaEntry? imeta;
  final String semanticLabel;
  final VoidCallback? onReply;
  final MediaViewerMoreAction? onMore;

  const _MessageImagePreview({
    required this.url,
    required this.imeta,
    required this.semanticLabel,
    required this.onReply,
    required this.onMore,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final heroTag = useMemoized(() => Object());
    final layout = _resolveImagePreviewLayout(context, imeta?.aspectRatio);
    final previewDecodeWidth = layout.width ?? _messageMediaMaxWidth(context);

    return Padding(
      padding: const EdgeInsets.only(top: Grid.half),
      child: GestureDetector(
        onTap: () => openImageViewer(
          context,
          imageUrl: url,
          heroTag: heroTag,
          semanticLabel: semanticLabel,
          previewDecodeWidth: previewDecodeWidth,
          aspectRatio: imeta?.aspectRatio,
          onReply: onReply,
          onMore: onMore,
        ),
        child: _MessageMediaPreviewFrame(
          previewKey: ValueKey('message-media-image-preview:$url'),
          backgroundColor: context.colors.surfaceContainerHighest,
          width: layout.width,
          height: layout.height,
          constraints: layout.constraints,
          child: MediaViewerHero(
            tag: heroTag,
            child: ClipRRect(
              borderRadius: BorderRadius.circular(Radii.md),
              child: MediaImage(
                url: url,
                decodeWidth: previewDecodeWidth,
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
                  MediaImage(
                    url: posterUrl,
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
  final Set<String> agentMentionPubkeys;
  final void Function(String pubkey)? onMentionTap;
  late final RegExp _exp = _buildPrefixPattern(
    prefix: '@',
    knownNames: _mentionAliases(mentionNames.values),
    genericTokenPattern: r'[A-Za-z0-9_][A-Za-z0-9_\u00A0-]*',
  );

  _MentionMd({
    required this.mentionNames,
    required this.agentMentionPubkeys,
    this.onMentionTap,
  });

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
    String? pubkey;
    for (final entry in mentionNames.entries) {
      final entryName = entry.value.toLowerCase();
      final firstName = entryName.split(RegExp(r'\s+')).first;
      if (entryName == name || firstName == name) {
        displayName = entry.value;
        pubkey = entry.key;
        break;
      }
    }

    final isAgent =
        pubkey != null && agentMentionPubkeys.contains(pubkey.toLowerCase());
    final pill = _MentionPill(
      label: displayName ?? raw.substring(1),
      isAgent: isAgent,
      textStyle: config.style,
    );

    return WidgetSpan(
      alignment: PlaceholderAlignment.baseline,
      baseline: TextBaseline.alphabetic,
      child: pubkey != null && onMentionTap != null
          ? GestureDetector(onTap: () => onMentionTap!(pubkey!), child: pill)
          : pill,
    );
  }
}

class _MentionPill extends StatelessWidget {
  final String label;
  final bool isAgent;
  final TextStyle? textStyle;

  const _MentionPill({
    required this.label,
    required this.isAgent,
    this.textStyle,
  });

  @override
  Widget build(BuildContext context) {
    final style =
        (textStyle ?? context.textTheme.bodyMedium)?.copyWith(
          color: context.colors.primary,
          fontWeight: FontWeight.w500,
          height: 1,
        ) ??
        TextStyle(
          color: context.colors.primary,
          fontWeight: FontWeight.w500,
          height: 1,
        );
    final fontSize = style.fontSize ?? 16;

    return Container(
      padding: const EdgeInsets.fromLTRB(
        Grid.half,
        Grid.quarter + 1,
        Grid.half,
        Grid.quarter,
      ),
      decoration: BoxDecoration(
        color: context.colors.primary.withValues(alpha: 0.1),
        borderRadius: BorderRadius.circular(Radii.sm),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          if (isAgent) ...[
            Icon(
              LucideIcons.bot,
              size: fontSize * 0.95,
              color: context.colors.primary,
            ),
            const SizedBox(width: Grid.quarter),
          ] else
            Transform.translate(
              offset: const Offset(0, -Grid.quarter),
              child: Text('@', style: style),
            ),
          Text(label, style: style),
        ],
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

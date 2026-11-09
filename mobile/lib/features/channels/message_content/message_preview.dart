part of '../message_content.dart';

/// A [maxLines] preview of a message for list rows such as the inbox and
/// search results.
///
/// The preview is one [Text.rich] built by [messagePreviewPieces], not
/// parsed Markdown: mentions, channels and custom emoji keep their chips,
/// links show their label and media shows its kind. Opening the row shows the
/// message in full.
class MessagePreview extends HookConsumerWidget {
  final String content;
  final Map<String, String> mentionNames;
  final Map<String, String> mentionLabels;
  final Set<String> agentMentionPubkeys;
  final List<List<String>> tags;
  final int maxLines;
  final TextStyle? baseStyle;
  final TextAlign? textAlign;

  const MessagePreview({
    super.key,
    required this.content,
    this.mentionNames = const {},
    this.mentionLabels = const {},
    this.agentMentionPubkeys = const {},
    this.tags = const [],
    this.maxLines = 2,
    this.baseStyle,
    this.textAlign,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final style =
        baseStyle ??
        context.textTheme.bodyMedium?.copyWith(color: context.colors.onSurface);
    final channelNames = ref.watch(channelIdsByNameProvider);
    final palette = ref.watch(customEmojiListProvider);
    final pieces = useMemoized(
      () {
        final source = messagePreviewSource(content, maxLines: maxLines);
        return messagePreviewPieces(
          normalizeBareLinks(source),
          maxLines: maxLines,
          mentionBindings: renderedMentionBindings(
            source,
            mentionNames,
            mentionedPubkeysFromTags(tags),
          ),
          mentionNames: mentionNames,
          mentionLabels: mentionLabels,
          customEmoji: _referencedCustomEmoji(
            source,
            _mergeCustomEmoji(customEmojiFromTags(tags), palette),
          ),
          channelNames: channelNames,
          imetaByUrl: {
            for (final entry in parseImetaTags(tags).entries)
              normalizeMarkdownDestination(entry.key): entry.value,
          },
        );
      },
      [
        content,
        tags,
        maxLines,
        mentionNames,
        mentionLabels,
        channelNames,
        palette,
      ],
    );
    if (pieces.isEmpty) return const SizedBox.shrink();

    final agents = {for (final key in agentMentionPubkeys) key.toLowerCase()};
    final mutedStyle = style?.copyWith(color: context.colors.onSurfaceVariant);
    return Text.rich(
      TextSpan(
        style: style,
        children: [
          for (final piece in pieces)
            switch (piece) {
              PreviewText(:final text) => TextSpan(text: text),
              PreviewMention(
                :final pubkey,
                :final label,
                :final semanticsLabel,
              ) =>
                WidgetSpan(
                  alignment: PlaceholderAlignment.baseline,
                  baseline: TextBaseline.alphabetic,
                  child: MessageMentionPill(
                    label: label,
                    semanticsLabel: semanticsLabel,
                    isAgent: agents.contains(pubkey),
                    textStyle: style,
                  ),
                ),
              PreviewEmoji(:final shortcode, :final url) => WidgetSpan(
                alignment: PlaceholderAlignment.middle,
                child: CustomEmojiImage(
                  shortcode: shortcode,
                  url: url,
                  size: kCustomEmojiInlineSize,
                ),
              ),
              PreviewChannel(:final name) => WidgetSpan(
                alignment: PlaceholderAlignment.baseline,
                baseline: TextBaseline.alphabetic,
                child: _TokenPill(
                  icon: BuzzIcons.hash,
                  semanticLabel: 'Channel $name',
                  text: name,
                  textStyle: style?.copyWith(fontWeight: FontWeight.w500),
                ),
              ),
              PreviewMedia(:final kind) => TextSpan(
                text: switch (kind) {
                  MessageMediaKind.image => 'Image',
                  MessageMediaKind.video => 'Video',
                  MessageMediaKind.audio => 'Audio',
                },
                style: mutedStyle,
              ),
            },
        ],
      ),
      textAlign: textAlign,
      maxLines: maxLines,
      overflow: TextOverflow.ellipsis,
    );
  }
}

/// The palette entries [content] names, so a large community palette does
/// not cost every row.
Map<String, String> _referencedCustomEmoji(
  String content,
  List<CustomEmoji> palette,
) {
  final referenced = {
    for (final match in _previewShortcodeScan.allMatches(content))
      match.group(1)!.toLowerCase(),
  };
  if (referenced.isEmpty) return const {};
  return {
    for (final emoji in palette)
      if (referenced.contains(emoji.shortcode)) emoji.shortcode: emoji.url,
  };
}

final _previewShortcodeScan = RegExp(
  r'(?=:([a-z0-9_-]+):)',
  caseSensitive: false,
);

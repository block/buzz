part of '../compose_bar.dart';

@immutable
class _ComposeDraftPayload {
  final String content;
  final List<List<String>> mediaTags;

  const _ComposeDraftPayload({required this.content, required this.mediaTags});

  factory _ComposeDraftPayload.fromDraft({
    required String text,
    required List<BlobDescriptor> attachments,
    required List<CustomEmoji> customEmoji,
  }) {
    var content = text;
    final mediaTags = <List<String>>[];
    for (final attachment in attachments) {
      mediaTags.add(attachment.toImetaTag());
      content += '\n${attachment.toMarkdownImage()}';
    }
    mediaTags.addAll(buildCustomEmojiTags(content, customEmoji));
    return _ComposeDraftPayload(content: content, mediaTags: mediaTags);
  }
}

List<BlobDescriptor> _withoutAttachment(
  List<BlobDescriptor> attachments,
  String url,
) {
  return [
    for (final attachment in attachments)
      if (attachment.url != url) attachment,
  ];
}

class _AttachmentStrip extends StatelessWidget {
  final List<BlobDescriptor> attachments;
  final int uploadingCount;
  final void Function(String url) onRemove;

  const _AttachmentStrip({
    required this.attachments,
    required this.uploadingCount,
    required this.onRemove,
  });

  @override
  Widget build(BuildContext context) {
    final thumbWidth = 72.0;
    final thumbHeight = 72.0;

    return SizedBox(
      height: thumbHeight,
      child: ListView.separated(
        scrollDirection: Axis.horizontal,
        itemCount: attachments.length + uploadingCount,
        separatorBuilder: (_, _) => const SizedBox(width: Grid.half),
        itemBuilder: (context, index) {
          if (index >= attachments.length) {
            return Container(
              width: thumbWidth,
              decoration: BoxDecoration(
                color: context.colors.surface,
                borderRadius: BorderRadius.circular(Radii.md),
                border: Border.all(color: context.colors.outlineVariant),
              ),
              child: const Center(
                child: CircularProgressIndicator(strokeWidth: 2),
              ),
            );
          }

          final attachment = attachments[index];
          final isVideo = attachment.type.startsWith('video/');
          final previewUrl = attachment.thumb ?? attachment.url;
          return Container(
            key: ValueKey('compose-attachment:${attachment.url}'),
            width: thumbWidth,
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(Radii.md),
              border: Border.all(color: context.colors.outlineVariant),
            ),
            child: Stack(
              fit: StackFit.expand,
              children: [
                ClipRRect(
                  borderRadius: BorderRadius.circular(Radii.md),
                  child: isVideo
                      ? ColoredBox(
                          color: Colors.black,
                          child: Center(
                            child: Icon(
                              LucideIcons.video,
                              color: Colors.white,
                              size: 24,
                            ),
                          ),
                        )
                      : MediaImage(
                          url: previewUrl,
                          fit: BoxFit.cover,
                          errorBuilder: (_, _, _) => ColoredBox(
                            color: context.colors.surface,
                            child: Icon(
                              LucideIcons.image,
                              color: context.colors.onSurfaceVariant,
                            ),
                          ),
                        ),
                ),
                Positioned(
                  top: Grid.quarter,
                  right: Grid.quarter,
                  child: SizedBox(
                    width: 24,
                    height: 24,
                    child: IconButton(
                      onPressed: () => onRemove(attachment.url),
                      tooltip: 'Remove attachment',
                      visualDensity: VisualDensity.compact,
                      style: IconButton.styleFrom(
                        backgroundColor: context.colors.surface.withValues(
                          alpha: 0.92,
                        ),
                        minimumSize: const Size(24, 24),
                        maximumSize: const Size(24, 24),
                        padding: EdgeInsets.zero,
                        tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                      ),
                      icon: Icon(
                        LucideIcons.x,
                        size: 14,
                        color: context.colors.onSurface,
                      ),
                    ),
                  ),
                ),
              ],
            ),
          );
        },
      ),
    );
  }
}

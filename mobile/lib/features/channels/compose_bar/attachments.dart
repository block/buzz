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

class _AttachmentTrigger extends StatelessWidget {
  final bool open;
  final VoidCallback onTap;

  const _AttachmentTrigger({required this.open, required this.onTap});

  @override
  Widget build(BuildContext context) {
    final duration = MediaQuery.disableAnimationsOf(context)
        ? Duration.zero
        : const Duration(milliseconds: 180);

    return SizedBox.square(
      dimension: 36,
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: context.colors.surface,
          shape: BoxShape.circle,
          border: Border.all(
            color: Colors.black.withValues(alpha: 0.04),
            width: 1,
          ),
        ),
        child: IconButton(
          tooltip: open ? 'Close attachments' : 'Add attachment',
          onPressed: onTap,
          padding: EdgeInsets.zero,
          visualDensity: VisualDensity.compact,
          icon: AnimatedRotation(
            duration: duration,
            curve: Curves.easeInOutCubic,
            turns: open ? 0.125 : 0,
            child: Icon(
              LucideIcons.plus,
              size: 20,
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ),
      ),
    );
  }
}

class _AttachmentMenu extends StatelessWidget {
  final VoidCallback onCamera;
  final VoidCallback onPhotos;
  final VoidCallback onVideo;
  final VoidCallback onFiles;

  const _AttachmentMenu({
    required this.onCamera,
    required this.onPhotos,
    required this.onVideo,
    required this.onFiles,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      width: 176,
      clipBehavior: Clip.antiAlias,
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(Radii.dialog),
        border: Border.all(
          color: Colors.black.withValues(alpha: 0.04),
          width: 1,
        ),
      ),
      padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          _AttachmentMenuItem(
            icon: LucideIcons.camera,
            label: 'Camera',
            onTap: onCamera,
          ),
          _AttachmentMenuItem(
            icon: LucideIcons.images,
            label: 'Photos',
            onTap: onPhotos,
          ),
          _AttachmentMenuItem(
            icon: LucideIcons.video,
            label: 'Video',
            onTap: onVideo,
          ),
          _AttachmentMenuItem(
            icon: LucideIcons.file,
            label: 'Files',
            onTap: onFiles,
          ),
        ],
      ),
    );
  }
}

class _AttachmentMenuItem extends StatelessWidget {
  final IconData icon;
  final String label;
  final VoidCallback onTap;

  const _AttachmentMenuItem({
    required this.icon,
    required this.label,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: 48,
      child: Tooltip(
        message: label,
        child: InkWell(
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.twelve),
            child: Row(
              children: [
                Icon(icon, size: 20, color: context.colors.onSurfaceVariant),
                const SizedBox(width: Grid.xxs),
                Text(
                  label,
                  style: context.textTheme.bodyLarge?.copyWith(
                    color: context.colors.onSurface,
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
        itemCount: attachments.length + (uploadingCount > 0 ? 1 : 0),
        separatorBuilder: (_, _) => const SizedBox(width: Grid.half),
        itemBuilder: (context, index) {
          if (index == attachments.length) {
            final label = uploadingCount == 1
                ? 'Uploading attachment…'
                : 'Uploading $uploadingCount attachments…';
            return Semantics(
              liveRegion: true,
              label: label,
              child: Container(
                key: const ValueKey('compose-upload-progress'),
                width: 128,
                decoration: BoxDecoration(
                  color: context.colors.surface,
                  borderRadius: BorderRadius.circular(Radii.md),
                  border: Border.all(color: context.colors.outlineVariant),
                ),
                padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.center,
                  children: [
                    SizedBox.square(
                      dimension: 18,
                      child: CircularProgressIndicator(
                        strokeWidth: 2,
                        color: context.colors.primary,
                      ),
                    ),
                    const SizedBox(width: Grid.half),
                    Flexible(
                      child: Text(
                        label,
                        maxLines: 2,
                        overflow: TextOverflow.ellipsis,
                        style: context.textTheme.labelSmall?.copyWith(
                          color: context.colors.onSurfaceVariant,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            );
          }

          final attachment = attachments[index];
          final isVideo = attachment.type.startsWith('video/');
          final isImage = attachment.type.startsWith('image/');
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
                      : isImage
                      ? MediaImage(
                          url: previewUrl,
                          fit: BoxFit.cover,
                          errorBuilder: (_, _, _) => ColoredBox(
                            color: context.colors.surface,
                            child: Icon(
                              LucideIcons.image,
                              color: context.colors.onSurfaceVariant,
                            ),
                          ),
                        )
                      : ColoredBox(
                          color: context.colors.surface,
                          child: Padding(
                            padding: const EdgeInsets.all(Grid.xxs),
                            child: Column(
                              mainAxisAlignment: MainAxisAlignment.center,
                              children: [
                                Icon(
                                  LucideIcons.file,
                                  color: context.colors.onSurfaceVariant,
                                ),
                                const SizedBox(height: Grid.quarter),
                                Text(
                                  attachment.filename ?? 'File',
                                  maxLines: 2,
                                  textAlign: TextAlign.center,
                                  overflow: TextOverflow.ellipsis,
                                  style: context.textTheme.labelSmall?.copyWith(
                                    color: context.colors.onSurfaceVariant,
                                  ),
                                ),
                              ],
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

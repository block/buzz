part of '../compose_bar.dart';

enum _AttachmentSurface { closed, menu, camera, photos }

const _attachmentMenuWidth = 176.0;
const _attachmentMenuHeight = 208.0;
const _attachmentExpandedHeight = 372.0;

class _AttachmentSurfacePanel extends HookWidget {
  final _AttachmentSurface surface;
  final Widget suggestionPanel;
  final VoidCallback onBack;
  final VoidCallback onCamera;
  final VoidCallback onPhotos;
  final VoidCallback onVideo;
  final VoidCallback onFiles;
  final Future<void> Function(XFile image) onCapture;
  final Future<List<XFile>> Function() onPickAllPhotos;
  final Future<void> Function(List<XFile> photos) onChoosePhotos;

  const _AttachmentSurfacePanel({
    super.key,
    required this.surface,
    required this.suggestionPanel,
    required this.onBack,
    required this.onCamera,
    required this.onPhotos,
    required this.onVideo,
    required this.onFiles,
    required this.onCapture,
    required this.onPickAllPhotos,
    required this.onChoosePhotos,
  });

  @override
  Widget build(BuildContext context) {
    if (surface == _AttachmentSurface.closed) return suggestionPanel;

    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final isExpanded =
        surface == _AttachmentSurface.camera ||
        surface == _AttachmentSurface.photos;
    final morphController = useAnimationController(
      initialValue: isExpanded ? 1 : 0,
      duration: reducedMotion
          ? Duration.zero
          : const Duration(milliseconds: 320),
      reverseDuration: reducedMotion
          ? Duration.zero
          : const Duration(milliseconds: 250),
    );
    final rawProgress = useAnimation(morphController);
    final renderedExpandedSurface = useState<_AttachmentSurface?>(
      isExpanded ? surface : null,
    );
    final latestSurface = useRef(surface);
    latestSurface.value = surface;

    useEffect(() {
      void disposeCollapsedContent(AnimationStatus status) {
        if (status == AnimationStatus.dismissed &&
            latestSurface.value == _AttachmentSurface.menu) {
          renderedExpandedSurface.value = null;
        }
      }

      morphController.addStatusListener(disposeCollapsedContent);
      return () =>
          morphController.removeStatusListener(disposeCollapsedContent);
    }, [morphController]);

    useEffect(() {
      if (isExpanded) {
        renderedExpandedSurface.value = surface;
        morphController.forward();
      } else {
        morphController.reverse();
      }
      return null;
    }, [isExpanded, morphController, surface]);

    final visibleExpandedSurface =
        renderedExpandedSurface.value ?? (isExpanded ? surface : null);
    final expandedContent = switch (visibleExpandedSurface) {
      _AttachmentSurface.camera => KeyedSubtree(
        key: const ValueKey('camera-preview'),
        child: _InlineCameraPreview(onClose: onBack, onCapture: onCapture),
      ),
      _AttachmentSurface.photos => KeyedSubtree(
        key: const ValueKey('photo-gallery'),
        child: _PhotoGalleryPicker(
          onBack: onBack,
          onPickAllPhotos: onPickAllPhotos,
          onChoosePhotos: onChoosePhotos,
        ),
      ),
      _AttachmentSurface.closed ||
      _AttachmentSurface.menu ||
      null => const SizedBox.shrink(),
    };

    double interval(double value, double begin, double end) {
      return ((value - begin) / (end - begin)).clamp(0.0, 1.0);
    }

    final menuOpacity = 1 - interval(rawProgress, 0.12, 0.38);
    final expandedOpacity = interval(rawProgress, 0.28, 0.65);
    final sizeProgress = morphController.status == AnimationStatus.reverse
        ? const Cubic(0.22, 1, 0.36, 1).transform(rawProgress)
        : Curves.easeInOutCubic.transform(rawProgress);

    return LayoutBuilder(
      builder: (context, constraints) {
        final expandedWidth = constraints.maxWidth;
        const expandedHeight = _attachmentExpandedHeight;
        final width =
            _attachmentMenuWidth +
            ((expandedWidth - _attachmentMenuWidth) * sizeProgress);
        final height =
            _attachmentMenuHeight +
            ((expandedHeight - _attachmentMenuHeight) * sizeProgress);
        final baseColor = context.colors.surfaceContainerHighest;
        final expandedColor =
            visibleExpandedSurface == _AttachmentSurface.camera
            ? Colors.black
            : baseColor;

        return Align(
          alignment: Alignment.topLeft,
          heightFactor: 1,
          child: SizedBox(
            width: width,
            height: height,
            child: DecoratedBox(
              decoration: BoxDecoration(
                color: Color.lerp(baseColor, expandedColor, sizeProgress),
                borderRadius: BorderRadius.circular(Radii.dialog),
                border: Border.all(
                  color: Colors.black.withValues(alpha: 0.04),
                  width: 1,
                ),
              ),
              child: ClipRRect(
                borderRadius: BorderRadius.circular(Radii.dialog),
                child: Material(
                  type: MaterialType.transparency,
                  child: Stack(
                    clipBehavior: Clip.hardEdge,
                    children: [
                      Positioned(
                        left: 0,
                        top: 0,
                        width: _attachmentMenuWidth,
                        height: _attachmentMenuHeight,
                        child: IgnorePointer(
                          ignoring: surface != _AttachmentSurface.menu,
                          child: Opacity(
                            opacity: menuOpacity,
                            child: _AttachmentMenu(
                              onCamera: onCamera,
                              onPhotos: onPhotos,
                              onVideo: onVideo,
                              onFiles: onFiles,
                            ),
                          ),
                        ),
                      ),
                      Positioned(
                        left: 0,
                        top: 0,
                        width: expandedWidth,
                        height: expandedHeight,
                        child: IgnorePointer(
                          ignoring: !isExpanded,
                          child: Opacity(
                            opacity: expandedOpacity,
                            child: expandedContent,
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

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
  final _AttachmentSurface surface;
  final bool formattingOpen;
  final ValueChanged<BuildContext> onTap;

  const _AttachmentTrigger({
    required this.surface,
    required this.formattingOpen,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final duration = MediaQuery.disableAnimationsOf(context)
        ? Duration.zero
        : const Duration(milliseconds: 240);

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
          tooltip: switch (surface) {
            _AttachmentSurface.closed =>
              formattingOpen ? 'Close formatting' : 'Add attachment',
            _AttachmentSurface.menu => 'Close attachments',
            _AttachmentSurface.camera ||
            _AttachmentSurface.photos => 'Back to attachment options',
          },
          onPressed: () => onTap(context),
          padding: EdgeInsets.zero,
          visualDensity: VisualDensity.compact,
          icon: AnimatedRotation(
            duration: duration,
            curve: Curves.easeOutBack,
            turns: surface == _AttachmentSurface.menu || formattingOpen
                ? 0.125
                : 0,
            child: AnimatedSwitcher(
              duration: duration,
              switchInCurve: Curves.easeOutBack,
              switchOutCurve: Curves.easeInOutCubic,
              transitionBuilder: (child, animation) => FadeTransition(
                opacity: animation,
                child: ScaleTransition(
                  scale: Tween<double>(begin: 0.92, end: 1).animate(animation),
                  child: child,
                ),
              ),
              child: Icon(
                switch (surface) {
                  _AttachmentSurface.camera => LucideIcons.camera,
                  _AttachmentSurface.photos => LucideIcons.images,
                  _AttachmentSurface.closed ||
                  _AttachmentSurface.menu => LucideIcons.plus,
                },
                key: ValueKey('attachment-trigger-${surface.name}'),
                size: 20,
                color: context.colors.onSurfaceVariant,
              ),
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
    return SizedBox(
      width: _attachmentMenuWidth,
      height: _attachmentMenuHeight,
      child: Padding(
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
              excludeSemantics: true,
              liveRegion: true,
              label: label,
              child: Container(
                key: const ValueKey('compose-upload-progress'),
                width: thumbWidth,
                decoration: BoxDecoration(
                  color: context.colors.surface,
                  borderRadius: BorderRadius.circular(Radii.md),
                  border: Border.all(color: context.colors.outlineVariant),
                ),
                child: Stack(
                  alignment: Alignment.center,
                  children: [
                    BuzzLoadingIndicator(
                      size: 34,
                      color: context.colors.primary,
                      semanticLabel: label,
                    ),
                    if (uploadingCount > 1)
                      PositionedDirectional(
                        top: Grid.quarter,
                        end: Grid.quarter,
                        child: Container(
                          key: const ValueKey('compose-upload-count'),
                          constraints: const BoxConstraints(
                            minWidth: 22,
                            minHeight: 22,
                          ),
                          alignment: Alignment.center,
                          decoration: BoxDecoration(
                            color: context.colors.primary,
                            shape: BoxShape.circle,
                          ),
                          padding: const EdgeInsets.all(3),
                          child: Text(
                            '$uploadingCount',
                            style: context.textTheme.labelSmall?.copyWith(
                              color: context.colors.onPrimary,
                              fontWeight: FontWeight.w700,
                            ),
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

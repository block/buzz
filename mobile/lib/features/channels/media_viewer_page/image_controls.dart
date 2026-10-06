part of '../media_viewer_page.dart';

class _MediaViewerBottomControls extends StatelessWidget {
  final List<MediaViewerImage> images;
  final int currentIndex;
  final ValueChanged<int> onSelect;
  final VoidCallback? onReply;
  final VoidCallback? onMore;

  const _MediaViewerBottomControls({
    required this.images,
    required this.currentIndex,
    required this.onSelect,
    required this.onReply,
    required this.onMore,
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(Grid.sm, Grid.xxs, Grid.sm, 0),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          _MediaViewerCircleButton(
            key: const ValueKey('message-media-image-viewer-reply-thread'),
            icon: LucideIcons.messageSquareReply,
            tooltip: 'Reply in thread',
            onPressed: onReply,
          ),
          const SizedBox(width: Grid.xxs),
          Expanded(
            child: images.length > 1
                ? Theme(
                    data: ThemeData.dark(),
                    child: PageIndicator(
                      key: const ValueKey(
                        'message-media-image-viewer-pagination',
                      ),
                      semanticLabel: 'Image',
                      containerHeight: 48,
                      count: images.length,
                      selected: currentIndex,
                      animateChanges: !MediaQuery.disableAnimationsOf(context),
                      onSelected: onSelect,
                    ),
                  )
                : const SizedBox(height: 56),
          ),
          const SizedBox(width: Grid.xxs),
          _MediaViewerCircleButton(
            key: const ValueKey('message-media-image-viewer-more-actions'),
            icon: LucideIcons.ellipsis,
            tooltip: 'More image actions',
            onPressed: onMore,
          ),
        ],
      ),
    );
  }
}

class _MediaViewerCircleButton extends StatelessWidget {
  final IconData icon;
  final String tooltip;
  final VoidCallback? onPressed;

  const _MediaViewerCircleButton({
    super.key,
    required this.icon,
    required this.tooltip,
    required this.onPressed,
  });

  @override
  Widget build(BuildContext context) {
    if (onPressed != null && defaultTargetPlatform == TargetPlatform.iOS) {
      return Theme(
        data: ThemeData.dark(),
        child: IosGlassNavigationButton(
          icon: icon == LucideIcons.x
              ? IosGlassNavigationIcon.close
              : icon == LucideIcons.ellipsis
              ? IosGlassNavigationIcon.more
              : IosGlassNavigationIcon.reply,
          semanticLabel: tooltip,
          foregroundColor: Colors.white,
          controlSize: 48,
          onPressed: onPressed,
        ),
      );
    }
    return SizedBox.square(
      dimension: 48,
      child: onPressed == null
          ? const SizedBox.shrink()
          : DecoratedBox(
              decoration: BoxDecoration(
                color: Colors.white.withValues(alpha: 0.16),
                shape: BoxShape.circle,
              ),
              child: IconButton(
                onPressed: onPressed,
                tooltip: tooltip,
                icon: Icon(icon, color: Colors.white, size: 20),
              ),
            ),
    );
  }
}

class _MediaViewerCloseButton extends StatelessWidget {
  const _MediaViewerCloseButton({
    super.key,
    required this.tooltip,
    required this.onPressed,
  });

  final String tooltip;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    if (Theme.of(context).platform == TargetPlatform.iOS) {
      return IosGlassNavigationButton(
        icon: IosGlassNavigationIcon.close,
        semanticLabel: tooltip,
        onPressed: onPressed,
        foregroundColor: Colors.white,
      );
    }
    return _MediaViewerCircleButton(
      icon: LucideIcons.x,
      tooltip: tooltip,
      onPressed: onPressed,
    );
  }
}

EdgeInsets _mediaViewerPadding(BuildContext context) {
  final viewPadding = MediaQuery.viewPaddingOf(context);
  return EdgeInsets.only(
    top: viewPadding.top + 48 + Grid.xxs,
    bottom: viewPadding.bottom + 56 + (Grid.xxs * 2),
  );
}

Size _imageViewerSize(Size viewport, double? aspectRatio) {
  if (aspectRatio == null || aspectRatio <= 0) {
    return viewport;
  }

  final safeAspectRatio = aspectRatio.clamp(0.05, 20.0).toDouble();
  if (viewport.aspectRatio > safeAspectRatio) {
    return Size(viewport.height * safeAspectRatio, viewport.height);
  }
  return Size(viewport.width, viewport.width / safeAspectRatio);
}

void _precacheViewerImages(
  BuildContext context,
  List<MediaViewerImage> images,
  int focusedIndex,
) {
  for (var index = focusedIndex - 2; index <= focusedIndex + 2; index++) {
    if (index < 0 || index >= images.length) {
      continue;
    }
    final provider = images[index].preloadProvider;
    if (provider == null) {
      continue;
    }
    unawaited(precacheImage(provider, context, onError: (_, _) {}));
  }
}

bool _hasImageTransform(Matrix4 transform) {
  final storage = transform.storage;
  for (var index = 0; index < storage.length; index++) {
    if ((storage[index] - _identityTransformStorage[index]).abs() >
        _identityTransformEpsilon) {
      return true;
    }
  }
  return false;
}

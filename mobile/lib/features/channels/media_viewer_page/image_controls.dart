part of '../media_viewer_page.dart';

class _MediaViewerBottomControls extends StatelessWidget {
  final List<MediaViewerImage> images;
  final int currentIndex;
  final ValueListenable<double> pagePosition;
  final ValueChanged<double> onScrubUpdate;
  final VoidCallback onScrubEnd;
  final ValueChanged<int> onSelect;
  final VoidCallback? onReply;
  final VoidCallback? onMore;

  const _MediaViewerBottomControls({
    required this.images,
    required this.currentIndex,
    required this.pagePosition,
    required this.onScrubUpdate,
    required this.onScrubEnd,
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
                ? _MediaViewerFilmstrip(
                    key: const ValueKey('message-media-image-viewer-filmstrip'),
                    images: images,
                    currentIndex: currentIndex,
                    pagePosition: pagePosition,
                    onScrubUpdate: onScrubUpdate,
                    onScrubEnd: onScrubEnd,
                    onSelect: onSelect,
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

class _MediaViewerFilmstrip extends StatelessWidget {
  final List<MediaViewerImage> images;
  final int currentIndex;
  final ValueListenable<double> pagePosition;
  final ValueChanged<double> onScrubUpdate;
  final VoidCallback onScrubEnd;
  final ValueChanged<int> onSelect;

  const _MediaViewerFilmstrip({
    super.key,
    required this.images,
    required this.currentIndex,
    required this.pagePosition,
    required this.onScrubUpdate,
    required this.onScrubEnd,
    required this.onSelect,
  });

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: 56,
      child: RepaintBoundary(
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onHorizontalDragUpdate: (details) =>
              onScrubUpdate(details.primaryDelta ?? 0),
          onHorizontalDragEnd: (_) => onScrubEnd(),
          child: ValueListenableBuilder<double>(
            valueListenable: pagePosition,
            builder: (context, position, _) {
              return LayoutBuilder(
                builder: (context, constraints) {
                  const compactWidth = 40.0;
                  const focusedWidth = 72.0;
                  const itemHeight = 52.0;
                  const spacing = Grid.half;
                  final clampedPosition = position
                      .clamp(0.0, images.length - 1.0)
                      .toDouble();
                  final widths = <double>[];
                  final proximities = <double>[];
                  final centers = <double>[];
                  var cursor = 0.0;

                  for (var index = 0; index < images.length; index++) {
                    final proximity = (1 - (index - clampedPosition).abs())
                        .clamp(0.0, 1.0)
                        .toDouble();
                    final width =
                        compactWidth +
                        ((focusedWidth - compactWidth) * proximity);
                    widths.add(width);
                    proximities.add(proximity);
                    centers.add(cursor + (width / 2));
                    cursor += width + spacing;
                  }

                  final lowerIndex = clampedPosition.floor();
                  final upperIndex = clampedPosition.ceil();
                  final fraction = clampedPosition - lowerIndex;
                  final lowerCenter = centers[lowerIndex];
                  final upperCenter = centers[upperIndex];
                  final focusCenter =
                      lowerCenter + ((upperCenter - lowerCenter) * fraction);
                  final viewportCenter = constraints.maxWidth / 2;

                  return Stack(
                    clipBehavior: Clip.hardEdge,
                    children: [
                      for (var index = 0; index < images.length; index++)
                        Positioned(
                          left:
                              viewportCenter +
                              centers[index] -
                              focusCenter -
                              (widths[index] / 2),
                          top: Grid.quarter,
                          width: widths[index],
                          height: itemHeight,
                          child: _MediaViewerFilmstripImage(
                            image: images[index],
                            index: index,
                            proximity: proximities[index],
                            selected: index == currentIndex,
                            onSelect: onSelect,
                          ),
                        ),
                    ],
                  );
                },
              );
            },
          ),
        ),
      ),
    );
  }
}

class _MediaViewerFilmstripImage extends StatelessWidget {
  final MediaViewerImage image;
  final int index;
  final double proximity;
  final bool selected;
  final ValueChanged<int> onSelect;

  const _MediaViewerFilmstripImage({
    required this.image,
    required this.index,
    required this.proximity,
    required this.selected,
    required this.onSelect,
  });

  @override
  Widget build(BuildContext context) {
    final borderWidth = 1 + (1.5 * proximity);
    return Semantics(
      button: true,
      selected: selected,
      label: selected
          ? 'Image ${index + 1}, selected'
          : 'Show image ${index + 1}',
      child: GestureDetector(
        key: ValueKey('message-media-image-viewer-thumbnail:$index'),
        onTap: () => onSelect(index),
        child: Container(
          height: 52,
          padding: EdgeInsets.all(borderWidth),
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(Radii.sm),
            border: Border.all(
              color: Colors.white.withValues(alpha: 0.28 + (0.72 * proximity)),
              width: borderWidth,
            ),
          ),
          child: ClipRRect(
            key: ValueKey('message-media-image-viewer-thumbnail-clip:$index'),
            borderRadius: BorderRadius.circular(Radii.sm - borderWidth),
            clipBehavior: Clip.antiAlias,
            child: Opacity(
              opacity: 0.62 + (0.38 * proximity),
              child: MediaImage(
                url: image.url,
                decodeWidth: 72,
                fit: BoxFit.cover,
                semanticLabel: image.semanticLabel,
                errorBuilder: (_, _, _) => const ColoredBox(
                  color: Color.fromRGBO(255, 255, 255, 0.12),
                  child: Icon(
                    LucideIcons.imageOff,
                    color: Colors.white70,
                    size: 18,
                  ),
                ),
              ),
            ),
          ),
        ),
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

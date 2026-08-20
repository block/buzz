import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/physics.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:path_provider/path_provider.dart';
import 'package:video_player/video_player.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import 'media_viewer_hero.dart';

export 'media_viewer_hero.dart';

part 'media_viewer_page/image_controls.dart';
part 'media_viewer_page/route_transition.dart';
part 'media_viewer_page/video_controls.dart';
part 'media_viewer_page/video_viewer.dart';

const _imageViewerPushDuration = Duration(milliseconds: 260);
const _imageViewerPopDuration = Duration(milliseconds: 170);
const _identityTransformEpsilon = 0.0001;
final List<double> _identityTransformStorage = List<double>.unmodifiable(
  Matrix4.identity().storage,
);

/// Opens message-specific actions for the currently visible image.
typedef MediaViewerMoreAction =
    void Function(BuildContext context, String imageUrl);

/// An image and its source Hero tag in a full-screen media gallery.
@immutable
class MediaViewerImage {
  /// The image URL.
  final String url;

  /// The shared-element transition tag for the source thumbnail.
  final Object heroTag;

  /// The accessible image description.
  final String? semanticLabel;

  /// The logical decode width already cached by the source thumbnail.
  final double? previewDecodeWidth;

  /// The image's intrinsic width-to-height ratio, when provided by metadata.
  final double? aspectRatio;

  /// A display-sized provider that can be warmed before this page is shown.
  final ImageProvider<Object>? preloadProvider;

  /// Creates a media-viewer image.
  const MediaViewerImage({
    required this.url,
    required this.heroTag,
    this.semanticLabel,
    this.previewDecodeWidth,
    this.aspectRatio,
    this.preloadProvider,
  });
}

PageRoute<void> buildImageViewerRoute({
  required String imageUrl,
  required Object heroTag,
  String? semanticLabel,
  double? previewDecodeWidth,
  double? aspectRatio,
  List<MediaViewerImage>? galleryItems,
  int initialIndex = 0,
  VoidCallback? onReply,
  MediaViewerMoreAction? onMore,
  bool disableAnimations = false,
}) {
  final images =
      galleryItems ??
      [
        MediaViewerImage(
          url: imageUrl,
          heroTag: heroTag,
          semanticLabel: semanticLabel,
          previewDecodeWidth: previewDecodeWidth,
          aspectRatio: aspectRatio,
        ),
      ];
  final safeInitialIndex = initialIndex.clamp(0, images.length - 1).toInt();
  return PageRouteBuilder<void>(
    transitionDuration: disableAnimations
        ? Duration.zero
        : _imageViewerPushDuration,
    reverseTransitionDuration: disableAnimations
        ? Duration.zero
        : _imageViewerPopDuration,
    pageBuilder: (context, animation, secondaryAnimation) =>
        MediaImageViewerPage(
          imageUrl: imageUrl,
          heroTag: heroTag,
          semanticLabel: semanticLabel,
          galleryItems: images,
          initialIndex: safeInitialIndex,
          onReply: onReply,
          onMore: onMore,
        ),
    transitionsBuilder: (context, animation, secondaryAnimation, child) =>
        _MediaViewerRouteTransition(animation: animation, child: child),
  );
}

void openImageViewer(
  BuildContext context, {
  required String imageUrl,
  required Object heroTag,
  String? semanticLabel,
  double? previewDecodeWidth,
  double? aspectRatio,
  List<MediaViewerImage>? galleryItems,
  int initialIndex = 0,
  VoidCallback? onReply,
  MediaViewerMoreAction? onMore,
}) {
  Navigator.of(context).push(
    buildImageViewerRoute(
      imageUrl: imageUrl,
      heroTag: heroTag,
      semanticLabel: semanticLabel,
      previewDecodeWidth: previewDecodeWidth,
      aspectRatio: aspectRatio,
      galleryItems: galleryItems,
      initialIndex: initialIndex,
      onReply: onReply,
      onMore: onMore,
      disableAnimations: MediaQuery.disableAnimationsOf(context),
    ),
  );
}

void openVideoViewer(
  BuildContext context, {
  required String videoUrl,
  String? posterUrl,
  VoidCallback? onReply,
}) {
  Navigator.of(context).push(
    PageRouteBuilder<void>(
      transitionDuration: MediaQuery.disableAnimationsOf(context)
          ? Duration.zero
          : _imageViewerPushDuration,
      reverseTransitionDuration: MediaQuery.disableAnimationsOf(context)
          ? Duration.zero
          : _imageViewerPopDuration,
      pageBuilder: (context, animation, secondaryAnimation) =>
          MediaVideoViewerPage(
            videoUrl: videoUrl,
            posterUrl: posterUrl,
            onReply: onReply,
          ),
      transitionsBuilder: (context, animation, secondaryAnimation, child) =>
          _MediaViewerRouteTransition(animation: animation, child: child),
    ),
  );
}

class MediaImageViewerPage extends HookConsumerWidget {
  final String imageUrl;
  final Object heroTag;
  final String? semanticLabel;
  final List<MediaViewerImage>? galleryItems;
  final int initialIndex;
  final VoidCallback? onReply;
  final MediaViewerMoreAction? onMore;

  const MediaImageViewerPage({
    super.key,
    required this.imageUrl,
    required this.heroTag,
    this.semanticLabel,
    this.galleryItems,
    this.initialIndex = 0,
    this.onReply,
    this.onMore,
  });

  static const _dismissThreshold = 100.0;
  static const _dismissVelocity = 700.0;
  static const _backgroundFadeDivisor = 300.0;
  static const _filmstripScrubExtent = 44.0;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final images = useMemoized(
      () =>
          galleryItems ??
          [
            MediaViewerImage(
              url: imageUrl,
              heroTag: heroTag,
              semanticLabel: semanticLabel,
            ),
          ],
      [galleryItems, imageUrl, heroTag, semanticLabel],
    );
    final safeInitialIndex = initialIndex.clamp(0, images.length - 1).toInt();
    final currentIndex = useState(safeInitialIndex);
    final pageController = usePageController(initialPage: safeInitialIndex);
    final pagePosition = useState(safeInitialIndex.toDouble());
    final transformationControllers = useMemoized(
      () => [
        for (var index = 0; index < images.length; index++)
          TransformationController(),
      ],
      [images],
    );
    final snapBackController = useAnimationController(
      duration: const Duration(milliseconds: 200),
    );
    final zoomResetController = useAnimationController(
      duration: const Duration(milliseconds: 180),
    );
    final zoomResetListener = useRef<VoidCallback?>(null);
    final fullResolutionIndices = useState<Set<int>>(<int>{});
    final isTransformed = useState(false);
    final disableHeroOnDismiss = useState(false);
    final dragOffset = useState(0.0);
    final isDragging = useState(false);

    void handlePagePositionChanged() {
      if (!pageController.hasClients) return;
      final nextPosition = pageController.page;
      if (nextPosition == null ||
          (nextPosition - pagePosition.value).abs() < 0.0001) {
        return;
      }
      pagePosition.value = nextPosition;
    }

    void handleTransformChanged(int index) {
      if (index != currentIndex.value) return;
      final nextIsTransformed = _hasImageTransform(
        transformationControllers[index].value,
      );
      if (nextIsTransformed == isTransformed.value) {
        return;
      }

      isTransformed.value = nextIsTransformed;
      if (nextIsTransformed && isDragging.value) {
        isDragging.value = false;
        dragOffset.value = 0;
      }
    }

    useEffect(() {
      pageController.addListener(handlePagePositionChanged);
      return () => pageController.removeListener(handlePagePositionChanged);
    }, [pageController]);

    useEffect(() {
      final listeners = <VoidCallback>[];
      for (var index = 0; index < transformationControllers.length; index++) {
        final controllerIndex = index;
        void listener() => handleTransformChanged(controllerIndex);
        listeners.add(listener);
        transformationControllers[index].addListener(listener);
      }
      return () {
        for (var index = 0; index < transformationControllers.length; index++) {
          transformationControllers[index]
            ..removeListener(listeners[index])
            ..dispose();
        }
      };
    }, [transformationControllers]);

    useEffect(() {
      var cancelled = false;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!cancelled && context.mounted) {
          _precacheViewerImages(context, images, currentIndex.value);
        }
      });
      return () => cancelled = true;
    }, [images]);

    useEffect(() {
      return () {
        final listener = zoomResetListener.value;
        if (listener != null) {
          zoomResetController.removeListener(listener);
        }
      };
    }, [zoomResetController]);

    void onPageChanged(int index) {
      _precacheViewerImages(context, images, index);
      currentIndex.value = index;
      isTransformed.value = _hasImageTransform(
        transformationControllers[index].value,
      );
      disableHeroOnDismiss.value = index != safeInitialIndex;
      dragOffset.value = 0;
      isDragging.value = false;
    }

    void onFilmstripScrubUpdate(double delta) {
      if (isTransformed.value || !pageController.hasClients) return;
      final position = pageController.position;
      final viewport = position.viewportDimension;
      if (viewport <= 0) return;
      final target =
          (pageController.offset - ((delta / _filmstripScrubExtent) * viewport))
              .clamp(position.minScrollExtent, position.maxScrollExtent)
              .toDouble();
      pageController.jumpTo(target);
    }

    void onFilmstripScrubEnd() {
      if (!pageController.hasClients) return;
      final targetPage = (pageController.page ?? currentIndex.value.toDouble())
          .round()
          .clamp(0, images.length - 1);
      if (MediaQuery.disableAnimationsOf(context)) {
        pageController.jumpToPage(targetPage);
        return;
      }
      pageController.animateToPage(
        targetPage,
        duration: const Duration(milliseconds: 180),
        curve: Curves.easeOutCubic,
      );
    }

    void upgradeToFullResolution(int index) {
      if (images[index].previewDecodeWidth == null ||
          fullResolutionIndices.value.contains(index)) {
        return;
      }
      fullResolutionIndices.value = {...fullResolutionIndices.value, index};
    }

    void onImageInteractionStart(int index, ScaleStartDetails details) {
      if (details.pointerCount > 1) {
        upgradeToFullResolution(index);
      }
      if (details.pointerCount == 1 && !isTransformed.value) {
        isDragging.value = true;
      }
    }

    void onImageInteractionUpdate(int index, ScaleUpdateDetails details) {
      if (details.pointerCount > 1 || details.scale != 1) {
        final needsFullResolution =
            images[index].previewDecodeWidth != null &&
            !fullResolutionIndices.value.contains(index);
        if (isDragging.value || needsFullResolution) {
          if (needsFullResolution) {
            fullResolutionIndices.value = {
              ...fullResolutionIndices.value,
              index,
            };
          }
          isDragging.value = false;
          dragOffset.value = 0;
        }
        return;
      }

      if (!isDragging.value || isTransformed.value) return;
      dragOffset.value = (dragOffset.value + details.focalPointDelta.dy).clamp(
        0.0,
        MediaQuery.sizeOf(context).height,
      );
    }

    void resetImageTransform(int index) {
      final controller = transformationControllers[index];
      if (!_hasImageTransform(controller.value)) {
        return;
      }

      final previousListener = zoomResetListener.value;
      if (previousListener != null) {
        zoomResetController.removeListener(previousListener);
      }
      zoomResetController.stop();

      if (MediaQuery.disableAnimationsOf(context)) {
        controller.value = Matrix4.identity();
        zoomResetListener.value = null;
        return;
      }

      final animation =
          Matrix4Tween(
            begin: Matrix4.copy(controller.value),
            end: Matrix4.identity(),
          ).animate(
            CurvedAnimation(
              parent: zoomResetController,
              curve: Curves.easeOutCubic,
            ),
          );
      void listener() => controller.value = animation.value;
      zoomResetListener.value = listener;
      zoomResetController
        ..reset()
        ..addListener(listener)
        ..forward();
    }

    bool canDismissWithHero() =>
        !isTransformed.value || disableHeroOnDismiss.value;

    Future<void> prepareHeroFallbackDismiss() async {
      if (canDismissWithHero()) {
        return;
      }
      disableHeroOnDismiss.value = true;
      await WidgetsBinding.instance.endOfFrame;
    }

    Future<void> dismiss() async {
      await prepareHeroFallbackDismiss();
      if (!context.mounted) {
        return;
      }
      Navigator.of(context).maybePop();
    }

    void animateSnapBack() {
      final tween = Tween<double>(begin: dragOffset.value, end: 0);

      void listener() {
        dragOffset.value = tween.evaluate(snapBackController);
      }

      snapBackController
        ..stop()
        ..reset()
        ..addListener(listener);
      snapBackController
          .animateWith(
            SpringSimulation(
              SpringDescription.withDurationAndBounce(
                duration: const Duration(milliseconds: 260),
                bounce: 0.14,
              ),
              0,
              1,
              0,
              snapToEnd: true,
            ),
          )
          .whenCompleteOrCancel(() {
            snapBackController.removeListener(listener);
          });
    }

    void finishVerticalDismiss(double velocity) {
      isDragging.value = false;
      if (dragOffset.value > _dismissThreshold || velocity > _dismissVelocity) {
        unawaited(dismiss());
      } else {
        animateSnapBack();
      }
    }

    void onImageInteractionEnd(ScaleEndDetails details) {
      if (!isDragging.value) return;
      finishVerticalDismiss(details.velocity.pixelsPerSecond.dy);
    }

    Future<void> replyInThread() async {
      final callback = onReply;
      if (callback == null) return;
      final route = ModalRoute.of(context);
      await dismiss();
      await route?.completed;
      callback();
    }

    void showMoreActions() {
      onMore?.call(context, images[currentIndex.value].url);
    }

    final viewportHeight = MediaQuery.sizeOf(context).height;
    final dragProgress = (dragOffset.value / viewportHeight).clamp(0.0, 1.0);
    final imageScale = 1 - (dragProgress * 0.1);
    final chromeOpacity = (1 - (dragOffset.value / 160)).clamp(0.0, 1.0);

    return PopScope<void>(
      canPop: canDismissWithHero(),
      onPopInvokedWithResult: (didPop, result) {
        if (didPop) {
          return;
        }
        unawaited(dismiss());
      },
      child: Scaffold(
        key: const ValueKey('message-media-image-viewer'),
        backgroundColor: Colors.black.withValues(
          alpha: (1 - (dragOffset.value.abs() / _backgroundFadeDivisor)).clamp(
            0.3,
            1.0,
          ),
        ),
        body: Stack(
          children: [
            Positioned.fill(
              child: Transform.translate(
                offset: Offset(0, dragOffset.value),
                child: Transform.scale(
                  scale: imageScale,
                  child: PageView.builder(
                    key: const ValueKey('message-media-image-viewer-pages'),
                    controller: pageController,
                    physics: isTransformed.value
                        ? const NeverScrollableScrollPhysics()
                        : const PageScrollPhysics(),
                    itemCount: images.length,
                    onPageChanged: onPageChanged,
                    itemBuilder: (context, index) {
                      final image = images[index];
                      final viewPadding = MediaQuery.viewPaddingOf(context);
                      return Padding(
                        padding: EdgeInsets.only(
                          top: viewPadding.top + 48 + Grid.xxs,
                          bottom: viewPadding.bottom + 56 + (Grid.xxs * 2),
                        ),
                        child: LayoutBuilder(
                          builder: (context, constraints) {
                            final viewerSize = _imageViewerSize(
                              Size(constraints.maxWidth, constraints.maxHeight),
                              image.aspectRatio,
                            );
                            return GestureDetector(
                              key: ValueKey(
                                'message-media-image-viewer-gesture:$index',
                              ),
                              behavior: HitTestBehavior.opaque,
                              onDoubleTap: () => resetImageTransform(index),
                              child: InteractiveViewer(
                                transformationController:
                                    transformationControllers[index],
                                onInteractionStart: (details) =>
                                    onImageInteractionStart(index, details),
                                onInteractionUpdate: (details) =>
                                    onImageInteractionUpdate(index, details),
                                onInteractionEnd: onImageInteractionEnd,
                                panEnabled: isTransformed.value,
                                scaleEnabled: true,
                                minScale: 1,
                                maxScale: 4,
                                boundaryMargin: const EdgeInsets.all(Grid.xxl),
                                clipBehavior: Clip.none,
                                child: Align(
                                  alignment: Alignment.center,
                                  child: SizedBox(
                                    width: viewerSize.width,
                                    height: viewerSize.height,
                                    child: HeroMode(
                                      key: index == safeInitialIndex
                                          ? const ValueKey(
                                              'message-media-image-viewer-hero-mode',
                                            )
                                          : ValueKey(
                                              'message-media-image-viewer-hero-mode-$index',
                                            ),
                                      enabled:
                                          !disableHeroOnDismiss.value &&
                                          index == safeInitialIndex,
                                      child: MediaViewerHero(
                                        tag: image.heroTag,
                                        child: MediaImage(
                                          key: ValueKey(
                                            'message-media-image-viewer-image:$index',
                                          ),
                                          url: image.url,
                                          decodeWidth:
                                              fullResolutionIndices.value
                                                  .contains(index)
                                              ? null
                                              : image.previewDecodeWidth,
                                          boundDecodeToLayout: false,
                                          fit: BoxFit.contain,
                                          semanticLabel: image.semanticLabel,
                                          errorBuilder: (_, _, _) =>
                                              const _MediaLoadFailure(
                                                message: 'Failed to load image',
                                                icon: LucideIcons.imageOff,
                                              ),
                                        ),
                                      ),
                                    ),
                                  ),
                                ),
                              ),
                            );
                          },
                        ),
                      );
                    },
                  ),
                ),
              ),
            ),
            PositionedDirectional(
              bottom: 0,
              start: 0,
              end: 0,
              child: Opacity(
                opacity: chromeOpacity,
                child: SafeArea(
                  child: _MediaViewerBottomControls(
                    images: images,
                    currentIndex: currentIndex.value,
                    pagePosition: pagePosition,
                    onScrubUpdate: onFilmstripScrubUpdate,
                    onScrubEnd: onFilmstripScrubEnd,
                    onSelect: (index) {
                      if (index == currentIndex.value) return;
                      if (MediaQuery.disableAnimationsOf(context)) {
                        pageController.jumpToPage(index);
                        return;
                      }
                      pageController.animateToPage(
                        index,
                        duration: const Duration(milliseconds: 220),
                        curve: Curves.easeOutCubic,
                      );
                    },
                    onReply: onReply == null
                        ? null
                        : () => unawaited(replyInThread()),
                    onMore: onMore == null ? null : showMoreActions,
                  ),
                ),
              ),
            ),
            PositionedDirectional(
              top: 0,
              end: Grid.sm,
              child: Opacity(
                opacity: chromeOpacity,
                child: SafeArea(
                  child: _MediaViewerCircleButton(
                    key: const ValueKey('message-media-image-viewer-close'),
                    icon: LucideIcons.x,
                    tooltip: 'Close image viewer',
                    onPressed: () => unawaited(dismiss()),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _MediaLoadFailure extends StatelessWidget {
  final String message;
  final IconData icon;

  const _MediaLoadFailure({required this.message, required this.icon});

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final showMessage = constraints.maxWidth >= 120;
        final iconSize = constraints.biggest.shortestSide
            .clamp(0.0, 36.0)
            .toDouble();

        return Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, color: Colors.white70, size: iconSize),
            if (showMessage) ...[
              const SizedBox(height: Grid.xxs),
              Text(
                message,
                style: context.textTheme.bodyMedium?.copyWith(
                  color: Colors.white70,
                ),
                textAlign: TextAlign.center,
              ),
            ],
          ],
        );
      },
    );
  }
}

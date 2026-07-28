import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/physics.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:video_player/video_player.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';

part 'media_viewer_page/image_controls.dart';
part 'media_viewer_page/route_transition.dart';

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

/// Keeps image-viewer shared-element motion consistent at every source.
class MediaViewerHero extends StatelessWidget {
  /// The identity shared by the inline image and full-screen image.
  final Object tag;

  /// The image rendered during and after the shared-element transition.
  final Widget child;

  /// Creates an image-viewer shared element.
  const MediaViewerHero({super.key, required this.tag, required this.child});

  @override
  Widget build(BuildContext context) {
    return Hero(
      tag: tag,
      createRectTween: (begin, end) => RectTween(begin: begin, end: end),
      flightShuttleBuilder:
          (
            flightContext,
            animation,
            flightDirection,
            fromHeroContext,
            toHeroContext,
          ) {
            final sourceHero = fromHeroContext.widget;
            final destinationHero = toHeroContext.widget;
            final sourceChild = sourceHero is Hero ? sourceHero.child : child;
            final destinationChild = destinationHero is Hero
                ? destinationHero.child
                : child;
            return _MediaViewerHeroFlight(
              animation: animation,
              sourceChild: sourceChild,
              destinationChild: destinationChild,
            );
          },
      child: child,
    );
  }
}

class _MediaViewerHeroFlight extends StatelessWidget {
  final Animation<double> animation;
  final Widget sourceChild;
  final Widget destinationChild;

  const _MediaViewerHeroFlight({
    required this.animation,
    required this.sourceChild,
    required this.destinationChild,
  });

  @override
  Widget build(BuildContext context) {
    final destinationOpacity = CurvedAnimation(
      parent: animation,
      curve: const Interval(0.18, 0.82, curve: Curves.easeInOutCubic),
    );
    return Stack(
      fit: StackFit.expand,
      children: [
        FadeTransition(
          opacity: ReverseAnimation(destinationOpacity),
          child: sourceChild,
        ),
        FadeTransition(opacity: destinationOpacity, child: destinationChild),
      ],
    );
  }
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
          MediaVideoViewerPage(videoUrl: videoUrl, posterUrl: posterUrl),
      transitionsBuilder: (context, animation, secondaryAnimation, child) =>
          _MediaViewerRouteTransition(animation: animation, child: child),
    ),
  );
}

// StatefulWidget retained: imperative gesture/animation controllers with
// listener lifecycle don't map cleanly to hooks (allowed exception).
class MediaImageViewerPage extends StatefulWidget {
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

  @override
  State<MediaImageViewerPage> createState() => _MediaImageViewerPageState();
}

class _MediaImageViewerPageState extends State<MediaImageViewerPage>
    with TickerProviderStateMixin {
  late final List<MediaViewerImage> _images;
  late final int _initialIndex;
  late final PageController _pageController;
  late final ValueNotifier<double> _pagePosition;
  late final List<TransformationController> _transformationControllers;
  late final List<VoidCallback> _transformationListeners;
  late final AnimationController _snapBackController;
  late final AnimationController _zoomResetController;
  VoidCallback? _zoomResetListener;
  final Set<int> _fullResolutionIndices = <int>{};
  late int _currentIndex;
  bool _isTransformed = false;
  bool _disableHeroOnDismiss = false;
  double _dragOffset = 0;
  bool _isDragging = false;

  static const _dismissThreshold = 100.0;
  static const _dismissVelocity = 700.0;
  static const _backgroundFadeDivisor = 300.0;
  static const _filmstripScrubExtent = 44.0;
  @override
  void initState() {
    super.initState();
    _images =
        widget.galleryItems ??
        [
          MediaViewerImage(
            url: widget.imageUrl,
            heroTag: widget.heroTag,
            semanticLabel: widget.semanticLabel,
          ),
        ];
    _initialIndex = widget.initialIndex.clamp(0, _images.length - 1).toInt();
    _currentIndex = _initialIndex;
    _pageController = PageController(initialPage: _initialIndex);
    _pagePosition = ValueNotifier(_initialIndex.toDouble());
    _pageController.addListener(_handlePagePositionChanged);
    _transformationControllers = [
      for (var index = 0; index < _images.length; index++)
        TransformationController(),
    ];
    _transformationListeners = [];
    for (var index = 0; index < _transformationControllers.length; index++) {
      final controllerIndex = index;
      void listener() => _handleTransformChanged(controllerIndex);
      _transformationListeners.add(listener);
      _transformationControllers[index].addListener(listener);
    }
    _snapBackController = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 200),
    );
    _zoomResetController = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 180),
    );
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _precacheViewerImages(context, _images, _currentIndex);
  }

  @override
  void dispose() {
    for (var index = 0; index < _transformationControllers.length; index++) {
      _transformationControllers[index].removeListener(
        _transformationListeners[index],
      );
      _transformationControllers[index].dispose();
    }
    _pageController.removeListener(_handlePagePositionChanged);
    _pageController.dispose();
    _pagePosition.dispose();
    _snapBackController.dispose();
    final zoomResetListener = _zoomResetListener;
    if (zoomResetListener != null) {
      _zoomResetController.removeListener(zoomResetListener);
    }
    _zoomResetController.dispose();
    super.dispose();
  }

  void _handlePagePositionChanged() {
    if (!_pageController.hasClients) return;
    final nextPosition = _pageController.page;
    if (nextPosition == null ||
        (nextPosition - _pagePosition.value).abs() < 0.0001) {
      return;
    }
    _pagePosition.value = nextPosition;
  }

  void _handleTransformChanged(int index) {
    if (index != _currentIndex) return;
    final isTransformed = _hasImageTransform(
      _transformationControllers[index].value,
    );
    if (isTransformed == _isTransformed) {
      return;
    }

    setState(() {
      _isTransformed = isTransformed;
      // If the user zooms in while dragging, cancel the drag.
      if (_isTransformed && _isDragging) {
        _isDragging = false;
        _dragOffset = 0;
      }
    });
  }

  void _onPageChanged(int index) {
    _precacheViewerImages(context, _images, index);
    setState(() {
      _currentIndex = index;
      _isTransformed = _hasImageTransform(
        _transformationControllers[index].value,
      );
      _disableHeroOnDismiss = index != _initialIndex;
      _dragOffset = 0;
      _isDragging = false;
    });
  }

  void _onFilmstripScrubUpdate(double delta) {
    if (_isTransformed || !_pageController.hasClients) return;
    final position = _pageController.position;
    final viewport = position.viewportDimension;
    if (viewport <= 0) return;
    final target =
        (_pageController.offset - ((delta / _filmstripScrubExtent) * viewport))
            .clamp(position.minScrollExtent, position.maxScrollExtent)
            .toDouble();
    _pageController.jumpTo(target);
  }

  void _onFilmstripScrubEnd() {
    if (!_pageController.hasClients) return;
    final targetPage = (_pageController.page ?? _currentIndex.toDouble())
        .round()
        .clamp(0, _images.length - 1);
    if (MediaQuery.disableAnimationsOf(context)) {
      _pageController.jumpToPage(targetPage);
      return;
    }
    _pageController.animateToPage(
      targetPage,
      duration: const Duration(milliseconds: 180),
      curve: Curves.easeOutCubic,
    );
  }

  void _onImageInteractionStart(int index, ScaleStartDetails details) {
    if (details.pointerCount > 1) {
      _upgradeToFullResolution(index);
    }
    if (details.pointerCount == 1 && !_isTransformed) {
      _isDragging = true;
    }
  }

  void _onImageInteractionUpdate(int index, ScaleUpdateDetails details) {
    if (details.pointerCount > 1 || details.scale != 1) {
      final needsFullResolution =
          _images[index].previewDecodeWidth != null &&
          !_fullResolutionIndices.contains(index);
      if (_isDragging || needsFullResolution) {
        setState(() {
          if (needsFullResolution) {
            _fullResolutionIndices.add(index);
          }
          _isDragging = false;
          _dragOffset = 0;
        });
      }
      return;
    }

    if (!_isDragging || _isTransformed) return;
    setState(() {
      _dragOffset = (_dragOffset + details.focalPointDelta.dy).clamp(
        0.0,
        MediaQuery.sizeOf(context).height,
      );
    });
  }

  void _upgradeToFullResolution(int index) {
    if (_images[index].previewDecodeWidth == null ||
        _fullResolutionIndices.contains(index)) {
      return;
    }
    setState(() => _fullResolutionIndices.add(index));
  }

  void _resetImageTransform(int index) {
    final controller = _transformationControllers[index];
    if (!_hasImageTransform(controller.value)) {
      return;
    }

    final previousListener = _zoomResetListener;
    if (previousListener != null) {
      _zoomResetController.removeListener(previousListener);
    }
    _zoomResetController.stop();

    if (MediaQuery.disableAnimationsOf(context)) {
      controller.value = Matrix4.identity();
      _zoomResetListener = null;
      return;
    }

    final animation =
        Matrix4Tween(
          begin: Matrix4.copy(controller.value),
          end: Matrix4.identity(),
        ).animate(
          CurvedAnimation(
            parent: _zoomResetController,
            curve: Curves.easeOutCubic,
          ),
        );
    void listener() => controller.value = animation.value;
    _zoomResetListener = listener;
    _zoomResetController
      ..reset()
      ..addListener(listener)
      ..forward();
  }

  void _onImageInteractionEnd(ScaleEndDetails details) {
    if (!_isDragging) return;
    _finishVerticalDismiss(details.velocity.pixelsPerSecond.dy);
  }

  void _finishVerticalDismiss(double velocity) {
    _isDragging = false;

    if (_dragOffset > _dismissThreshold || velocity > _dismissVelocity) {
      unawaited(_dismiss());
    } else {
      _animateSnapBack();
    }
  }

  void _animateSnapBack() {
    final startOffset = _dragOffset;
    final tween = Tween<double>(begin: startOffset, end: 0);

    void listener() {
      setState(() {
        _dragOffset = tween.evaluate(_snapBackController);
      });
    }

    _snapBackController
      ..stop()
      ..reset()
      ..addListener(listener);
    _snapBackController
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
          _snapBackController.removeListener(listener);
        });
  }

  bool get _canDismissWithHero => !_isTransformed || _disableHeroOnDismiss;

  Future<void> _prepareHeroFallbackDismiss() async {
    if (_canDismissWithHero) {
      return;
    }

    setState(() {
      _disableHeroOnDismiss = true;
    });

    await WidgetsBinding.instance.endOfFrame;
  }

  Future<void> _dismiss() async {
    await _prepareHeroFallbackDismiss();
    if (!mounted) {
      return;
    }
    Navigator.of(context).maybePop();
  }

  Future<void> _replyInThread() async {
    final onReply = widget.onReply;
    if (onReply == null) return;
    final route = ModalRoute.of(context);
    await _dismiss();
    await route?.completed;
    onReply();
  }

  void _showMoreActions() {
    widget.onMore?.call(context, _images[_currentIndex].url);
  }

  @override
  Widget build(BuildContext context) {
    final viewportHeight = MediaQuery.sizeOf(context).height;
    final dragProgress = (_dragOffset / viewportHeight).clamp(0.0, 1.0);
    final imageScale = 1 - (dragProgress * 0.1);
    final chromeOpacity = (1 - (_dragOffset / 160)).clamp(0.0, 1.0);

    return PopScope<void>(
      canPop: _canDismissWithHero,
      onPopInvokedWithResult: (didPop, result) {
        if (didPop) {
          return;
        }
        unawaited(_dismiss());
      },
      child: Scaffold(
        key: const ValueKey('message-media-image-viewer'),
        backgroundColor: Colors.black.withValues(
          alpha: (1 - (_dragOffset.abs() / _backgroundFadeDivisor)).clamp(
            0.3,
            1.0,
          ),
        ),
        body: Stack(
          children: [
            Positioned.fill(
              child: Transform.translate(
                offset: Offset(0, _dragOffset),
                child: Transform.scale(
                  scale: imageScale,
                  child: PageView.builder(
                    key: const ValueKey('message-media-image-viewer-pages'),
                    controller: _pageController,
                    physics: _isTransformed
                        ? const NeverScrollableScrollPhysics()
                        : const PageScrollPhysics(),
                    itemCount: _images.length,
                    onPageChanged: _onPageChanged,
                    itemBuilder: (context, index) {
                      final image = _images[index];
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
                              onDoubleTap: () => _resetImageTransform(index),
                              child: InteractiveViewer(
                                transformationController:
                                    _transformationControllers[index],
                                onInteractionStart: (details) =>
                                    _onImageInteractionStart(index, details),
                                onInteractionUpdate: (details) =>
                                    _onImageInteractionUpdate(index, details),
                                onInteractionEnd: _onImageInteractionEnd,
                                panEnabled: _isTransformed,
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
                                      key: index == _initialIndex
                                          ? const ValueKey(
                                              'message-media-image-viewer-hero-mode',
                                            )
                                          : ValueKey(
                                              'message-media-image-viewer-hero-mode-$index',
                                            ),
                                      enabled:
                                          !_disableHeroOnDismiss &&
                                          index == _initialIndex,
                                      child: MediaViewerHero(
                                        tag: image.heroTag,
                                        child: MediaImage(
                                          key: ValueKey(
                                            'message-media-image-viewer-image:$index',
                                          ),
                                          url: image.url,
                                          decodeWidth:
                                              _fullResolutionIndices.contains(
                                                index,
                                              )
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
                    images: _images,
                    currentIndex: _currentIndex,
                    pagePosition: _pagePosition,
                    onScrubUpdate: _onFilmstripScrubUpdate,
                    onScrubEnd: _onFilmstripScrubEnd,
                    onSelect: (index) {
                      if (index == _currentIndex) return;
                      _pageController.animateToPage(
                        index,
                        duration: MediaQuery.disableAnimationsOf(context)
                            ? Duration.zero
                            : const Duration(milliseconds: 220),
                        curve: Curves.easeOutCubic,
                      );
                    },
                    onReply: widget.onReply == null
                        ? null
                        : () => unawaited(_replyInThread()),
                    onMore: widget.onMore == null ? null : _showMoreActions,
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
                    onPressed: () => unawaited(_dismiss()),
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

// StatefulWidget retained: owns a VideoPlayerController with async init and
// disposal — kept imperative deliberately (allowed exception).
class MediaVideoViewerPage extends StatefulWidget {
  final String videoUrl;
  final String? posterUrl;

  const MediaVideoViewerPage({
    super.key,
    required this.videoUrl,
    this.posterUrl,
  });

  @override
  State<MediaVideoViewerPage> createState() => _MediaVideoViewerPageState();
}

class _MediaVideoViewerPageState extends State<MediaVideoViewerPage> {
  late final VideoPlayerController _controller;
  late final Future<void> _initializeFuture;
  String? _error;

  @override
  void initState() {
    super.initState();
    _controller = VideoPlayerController.networkUrl(
      Uri.parse(widget.videoUrl),
      httpHeaders: mediaGetHeadersForContext(context, widget.videoUrl),
    );
    _initializeFuture = _controller
        .initialize()
        .then((_) async {
          await _controller.play();
          if (mounted) {
            setState(() {});
          }
        })
        .catchError((Object error) {
          if (mounted) {
            setState(() {
              _error = error.toString();
            });
          }
        });
  }

  @override
  void dispose() {
    unawaited(_controller.dispose());
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      key: const ValueKey('message-media-video-viewer'),
      backgroundColor: Colors.black,
      body: Stack(
        children: [
          Positioned.fill(
            child: SafeArea(
              child: Center(
                child: FutureBuilder<void>(
                  future: _initializeFuture,
                  builder: (context, snapshot) {
                    if (_error != null || snapshot.hasError) {
                      return const _MediaLoadFailure(
                        message: 'Failed to load video',
                        icon: LucideIcons.videoOff,
                      );
                    }

                    if (!_controller.value.isInitialized) {
                      return _VideoLoadingPoster(posterUrl: widget.posterUrl);
                    }

                    return Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        AspectRatio(
                          aspectRatio: _controller.value.aspectRatio,
                          child: VideoPlayer(_controller),
                        ),
                        const SizedBox(height: Grid.sm),
                        _VideoTransportBar(controller: _controller),
                      ],
                    );
                  },
                ),
              ),
            ),
          ),
          PositionedDirectional(
            top: Grid.sm,
            end: Grid.sm,
            child: SafeArea(
              child: DecoratedBox(
                decoration: const BoxDecoration(
                  color: Color.fromRGBO(0, 0, 0, 0.56),
                  shape: BoxShape.circle,
                ),
                child: IconButton(
                  key: const ValueKey('message-media-video-viewer-close'),
                  onPressed: () => Navigator.of(context).maybePop(),
                  tooltip: 'Close video viewer',
                  icon: const Icon(LucideIcons.x, color: Colors.white),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _VideoLoadingPoster extends StatelessWidget {
  final String? posterUrl;

  const _VideoLoadingPoster({required this.posterUrl});

  @override
  Widget build(BuildContext context) {
    return AspectRatio(
      aspectRatio: 16 / 9,
      child: Stack(
        fit: StackFit.expand,
        children: [
          if (posterUrl != null)
            MediaImage(
              url: posterUrl!,
              fit: BoxFit.cover,
              errorBuilder: (_, _, _) => _videoPlaceholder(context),
            )
          else
            _videoPlaceholder(context),
          const ColoredBox(color: Color.fromRGBO(0, 0, 0, 0.24)),
          const Center(
            child: CircularProgressIndicator(
              strokeWidth: 3,
              color: Colors.white,
            ),
          ),
        ],
      ),
    );
  }

  Widget _videoPlaceholder(BuildContext context) {
    return ColoredBox(
      color: context.colors.surfaceContainerHighest,
      child: Icon(
        LucideIcons.video,
        size: 40,
        color: context.colors.onSurfaceVariant,
      ),
    );
  }
}

class _VideoTransportBar extends HookWidget {
  final VideoPlayerController controller;

  const _VideoTransportBar({required this.controller});

  @override
  Widget build(BuildContext context) {
    useListenable(controller);
    final value = controller.value;
    final durationMs = value.duration.inMilliseconds;
    final positionMs = value.position.inMilliseconds.clamp(0, durationMs);

    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        IconButton(
          onPressed: () {
            if (value.isPlaying) {
              controller.pause();
            } else {
              controller.play();
            }
          },
          tooltip: value.isPlaying ? 'Pause video' : 'Play video',
          icon: Icon(
            value.isPlaying ? LucideIcons.pause : LucideIcons.play,
            color: Colors.white,
          ),
        ),
        SizedBox(
          width: 220,
          child: Slider(
            value: durationMs == 0 ? 0 : positionMs.toDouble(),
            min: 0,
            max: durationMs == 0 ? 1 : durationMs.toDouble(),
            onChanged: durationMs == 0
                ? null
                : (next) =>
                      controller.seekTo(Duration(milliseconds: next.round())),
          ),
        ),
      ],
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

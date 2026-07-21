import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:video_player/video_player.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';

const _imageViewerPushDuration = Duration(milliseconds: 280);
const _imageViewerPopDuration = Duration(milliseconds: 220);
const _imageViewerTransitionOffset = Offset(0, 0.08);
const _identityTransformEpsilon = 0.0001;
final List<double> _identityTransformStorage = List<double>.unmodifiable(
  Matrix4.identity().storage,
);

PageRoute<void> buildImageViewerRoute({
  required String imageUrl,
  required Object heroTag,
  String? semanticLabel,
}) {
  return PageRouteBuilder<void>(
    transitionDuration: _imageViewerPushDuration,
    reverseTransitionDuration: _imageViewerPopDuration,
    pageBuilder: (context, animation, secondaryAnimation) =>
        MediaImageViewerPage(
          imageUrl: imageUrl,
          heroTag: heroTag,
          semanticLabel: semanticLabel,
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
}) {
  Navigator.of(context).push(
    buildImageViewerRoute(
      imageUrl: imageUrl,
      heroTag: heroTag,
      semanticLabel: semanticLabel,
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
      transitionDuration: _imageViewerPushDuration,
      reverseTransitionDuration: _imageViewerPopDuration,
      pageBuilder: (context, animation, secondaryAnimation) =>
          MediaVideoViewerPage(videoUrl: videoUrl, posterUrl: posterUrl),
      transitionsBuilder: (context, animation, secondaryAnimation, child) =>
          _MediaViewerRouteTransition(animation: animation, child: child),
    ),
  );
}

class _MediaViewerRouteTransition extends StatelessWidget {
  final Animation<double> animation;
  final Widget child;

  const _MediaViewerRouteTransition({
    required this.animation,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    final fade = CurvedAnimation(
      parent: animation,
      curve: Curves.easeOut,
      reverseCurve: Curves.easeIn,
    );
    final slide = CurvedAnimation(
      parent: animation,
      curve: Curves.easeOutCubic,
      reverseCurve: Curves.easeInCubic,
    );

    return FadeTransition(
      opacity: fade,
      child: SlideTransition(
        position: Tween<Offset>(
          begin: _imageViewerTransitionOffset,
          end: Offset.zero,
        ).animate(slide),
        child: child,
      ),
    );
  }
}

// StatefulWidget retained: imperative gesture/animation controllers with
// listener lifecycle don't map cleanly to hooks (allowed exception).
class MediaImageViewerPage extends StatefulWidget {
  final String imageUrl;
  final Object heroTag;
  final String? semanticLabel;

  const MediaImageViewerPage({
    super.key,
    required this.imageUrl,
    required this.heroTag,
    this.semanticLabel,
  });

  @override
  State<MediaImageViewerPage> createState() => _MediaImageViewerPageState();
}

class _MediaImageViewerPageState extends State<MediaImageViewerPage>
    with SingleTickerProviderStateMixin {
  late final TransformationController _transformationController;
  late final AnimationController _snapBackController;
  bool _isTransformed = false;
  bool _disableHeroOnDismiss = false;
  double _dragOffset = 0;
  bool _isDragging = false;

  static const _dismissThreshold = 100.0;
  static const _backgroundFadeDivisor = 300.0;

  @override
  void initState() {
    super.initState();
    _transformationController = TransformationController();
    _transformationController.addListener(_handleTransformChanged);
    _snapBackController = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 200),
    );
  }

  @override
  void dispose() {
    _transformationController.removeListener(_handleTransformChanged);
    _transformationController.dispose();
    _snapBackController.dispose();
    super.dispose();
  }

  void _handleTransformChanged() {
    final isTransformed = _hasImageTransform(_transformationController.value);
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

  void _onInteractionStart(ScaleStartDetails details) {
    if (!_isTransformed && details.pointerCount == 1) {
      _isDragging = true;
    }
  }

  void _onInteractionUpdate(ScaleUpdateDetails details) {
    if (_isDragging && !_isTransformed) {
      setState(() {
        _dragOffset += details.focalPointDelta.dy;
      });
    }
  }

  void _onInteractionEnd(ScaleEndDetails details) {
    if (!_isDragging) return;
    _isDragging = false;

    if (_dragOffset.abs() > _dismissThreshold) {
      _dismiss();
    } else {
      _animateSnapBack();
    }
  }

  void _animateSnapBack() {
    final startOffset = _dragOffset;
    final tween = Tween<double>(begin: startOffset, end: 0);
    final curved = CurvedAnimation(
      parent: _snapBackController,
      curve: Curves.easeOut,
    );

    void listener() {
      setState(() {
        _dragOffset = tween.evaluate(curved);
      });
    }

    _snapBackController
      ..reset()
      ..addListener(listener);
    _snapBackController.forward().whenCompleteOrCancel(() {
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

  @override
  Widget build(BuildContext context) {
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
                child: InteractiveViewer(
                  transformationController: _transformationController,
                  onInteractionStart: _onInteractionStart,
                  onInteractionUpdate: _onInteractionUpdate,
                  onInteractionEnd: _onInteractionEnd,
                  minScale: 1,
                  maxScale: 4,
                  child: Center(
                    child: HeroMode(
                      key: const ValueKey(
                        'message-media-image-viewer-hero-mode',
                      ),
                      enabled: !_disableHeroOnDismiss,
                      child: Hero(
                        tag: widget.heroTag,
                        child: MediaImage(
                          url: widget.imageUrl,
                          boundDecodeToLayout: false,
                          fit: BoxFit.contain,
                          semanticLabel: widget.semanticLabel,
                          errorBuilder: (_, _, _) => const _MediaLoadFailure(
                            message: 'Failed to load image',
                            icon: LucideIcons.imageOff,
                          ),
                        ),
                      ),
                    ),
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
                    key: const ValueKey('message-media-image-viewer-close'),
                    onPressed: _dismiss,
                    tooltip: 'Close image viewer',
                    icon: const Icon(LucideIcons.x, color: Colors.white),
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
          const Center(child: CircularProgressIndicator()),
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

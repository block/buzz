part of '../media_viewer_page.dart';

class _VideoViewerBottomControls extends StatelessWidget {
  const _VideoViewerBottomControls({
    required this.controller,
    required this.onReply,
    required this.onInteractionStart,
    required this.onInteractionEnd,
  });

  final VideoPlayerController? controller;
  final VoidCallback? onReply;
  final VoidCallback onInteractionStart;
  final VoidCallback onInteractionEnd;

  @override
  Widget build(BuildContext context) {
    final readyController = controller?.value.isInitialized == true
        ? controller
        : null;
    return Padding(
      padding: const EdgeInsets.fromLTRB(Grid.xs, Grid.xxs, Grid.xs, Grid.xxs),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (readyController != null)
            _VideoTransportBar(
              controller: readyController,
              onReply: onReply,
              onInteractionStart: onInteractionStart,
              onInteractionEnd: onInteractionEnd,
            ),
          if (readyController == null && onReply != null) ...[
            const SizedBox(height: Grid.xxs),
            Align(
              alignment: AlignmentDirectional.centerStart,
              child: _MediaViewerCircleButton(
                key: const ValueKey('message-media-video-viewer-reply-thread'),
                icon: LucideIcons.messageSquareReply,
                tooltip: 'Reply in thread',
                onPressed: onReply,
              ),
            ),
          ],
        ],
      ),
    );
  }
}

class _VideoTransportBar extends HookConsumerWidget {
  const _VideoTransportBar({
    required this.controller,
    required this.onReply,
    required this.onInteractionStart,
    required this.onInteractionEnd,
  });

  final VideoPlayerController controller;
  final VoidCallback? onReply;
  final VoidCallback onInteractionStart;
  final VoidCallback onInteractionEnd;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(controller);
    final value = controller.value;
    final durationMs = value.duration.inMilliseconds;
    final positionMs = value.position.inMilliseconds.clamp(0, durationMs);
    final hasDuration = durationMs > 0;
    final scrubPosition = useState<double?>(null);
    final mutedVolume = useRef(1.0);
    final scrubGeneration = useRef(0);

    void beginScrub(double next) {
      scrubGeneration.value++;
      ScaffoldMessenger.of(context).hideCurrentSnackBar();
      onInteractionStart();
      scrubPosition.value = next;
    }

    Future<void> finishScrub(double next) async {
      final generation = scrubGeneration.value;
      bool isCurrent() =>
          context.mounted && generation == scrubGeneration.value;
      try {
        await controller.seekTo(Duration(milliseconds: next.round()));
      } catch (error) {
        debugPrint('[VideoViewer] seek failed: $error');
        if (context.mounted && isCurrent()) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(
              content: const Text('Could not seek in this video.'),
              action: SnackBarAction(
                label: 'Retry',
                onPressed: () {
                  if (!isCurrent()) return;
                  beginScrub(next);
                  unawaited(finishScrub(next));
                },
              ),
            ),
          );
        }
      } finally {
        if (isCurrent()) {
          scrubPosition.value = null;
          onInteractionEnd();
        }
      }
    }

    final displayedMs = scrubPosition.value?.round() ?? positionMs;
    final currentTime = _formatVideoDuration(
      Duration(milliseconds: displayedMs),
    );
    final totalTime = _formatVideoDuration(value.duration);
    final timeStyle = context.textTheme.labelSmall!.copyWith(
      color: Colors.white,
      fontFeatures: const [FontFeature.tabularFigures()],
    );
    final speedStyle = context.textTheme.labelMedium!.copyWith(
      color: Colors.white,
    );
    final textScaler = MediaQuery.textScalerOf(context);
    final textDirection = Directionality.of(context);
    final labelSizes = useMemoized(() {
      Size measure(String text, TextStyle style) {
        final painter = TextPainter(
          text: TextSpan(text: text, style: style),
          textDirection: textDirection,
          textScaler: textScaler,
        )..layout();
        final size = painter.size;
        painter.dispose();
        return size;
      }

      return (
        time: measure(totalTime, timeStyle),
        speed: measure('0.75×', speedStyle),
      );
    }, [totalTime, timeStyle, speedStyle, textScaler, textDirection]);
    final timeline = Expanded(
      child: SliderTheme(
        data: SliderTheme.of(context).copyWith(
          activeTrackColor: Colors.white,
          inactiveTrackColor: Colors.white.withValues(alpha: 0.16),
          thumbColor: Colors.white,
          overlayColor: Colors.white.withValues(alpha: 0.12),
          trackHeight: 4,
          thumbShape: const _VideoTimelineThumb(),
          trackShape: const RoundedRectSliderTrackShape(),
          overlayShape: const RoundSliderOverlayShape(overlayRadius: 16),
        ),
        child: SizedBox(
          height: 48,
          child: Slider(
            key: const ValueKey('message-media-video-viewer-timeline'),
            padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
            label: _formatVideoDuration(Duration(milliseconds: displayedMs)),
            semanticFormatterCallback: (next) =>
                '${_formatVideoDuration(Duration(milliseconds: next.round()))} of ${_formatVideoDuration(value.duration)}',
            value: hasDuration
                ? displayedMs.clamp(0, durationMs).toDouble()
                : 0,
            min: 0,
            max: hasDuration ? durationMs.toDouble() : 1,
            onChangeStart: hasDuration ? beginScrub : null,
            onChanged: hasDuration
                ? (next) {
                    scrubPosition.value = next;
                  }
                : null,
            onChangeEnd: hasDuration
                ? (next) => unawaited(finishScrub(next))
                : null,
          ),
        ),
      ),
    );
    final playPause = IconButton(
      key: const ValueKey('message-media-video-viewer-play-pause'),
      onPressed: () {
        onInteractionEnd();
        if (value.isPlaying) {
          unawaited(controller.pause());
        } else {
          unawaited(controller.play());
        }
      },
      tooltip: value.isPlaying ? 'Pause video' : 'Play video',
      icon: Icon(
        value.isPlaying ? LucideIcons.pause : LucideIcons.play,
        color: Colors.white,
        size: 20,
      ),
    );
    const speeds = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
    final nextSpeed =
        speeds[(speeds.indexOf(value.playbackSpeed) + 1) % speeds.length];
    String speedLabel(double speed) =>
        '${speed.toString().replaceFirst(RegExp(r'\.0$'), '')}×';
    final speed = Tooltip(
      message:
          'Playback speed: ${speedLabel(value.playbackSpeed)}. Tap for ${speedLabel(nextSpeed)}',
      child: TextButton(
        key: const ValueKey('message-media-video-viewer-speed'),
        onPressed: () {
          onInteractionEnd();
          unawaited(controller.setPlaybackSpeed(nextSpeed));
        },
        style: TextButton.styleFrom(
          foregroundColor: Colors.white,
          padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
        ),
        child: Text(speedLabel(value.playbackSpeed), style: speedStyle),
      ),
    );
    final mute = IconButton(
      key: const ValueKey('message-media-video-viewer-mute'),
      tooltip: value.volume == 0 ? 'Unmute video' : 'Mute video',
      onPressed: () {
        onInteractionEnd();
        if (value.volume > 0) mutedVolume.value = value.volume;
        unawaited(
          controller.setVolume(value.volume == 0 ? mutedVolume.value : 0),
        );
      },
      icon: Icon(
        value.volume == 0 ? LucideIcons.volumeX : LucideIcons.volume2,
        color: Colors.white,
        size: 20,
      ),
    );

    return LayoutBuilder(
      builder: (context, constraints) {
        final speedWidth = (labelSizes.speed.width + Grid.xxs * 2).clamp(
          48.0,
          double.infinity,
        );
        final height = (labelSizes.speed.height + Grid.xxs * 2).clamp(
          48.0,
          double.infinity,
        );
        // Keep both timestamps with the timeline. At large accessibility text
        // sizes the transport row can scroll without shrinking the text.
        final minimumWidth = 48 + 64 + labelSizes.time.width * 2 + Grid.xxs;
        final rowWidth = constraints.maxWidth.clamp(
          minimumWidth,
          double.infinity,
        );
        final transport = _VideoControlSurface(
          child: SizedBox(
            width: rowWidth,
            height: height,
            child: Padding(
              padding: const EdgeInsetsDirectional.only(end: Grid.xxs),
              child: Row(
                children: [
                  playPause,
                  Text(
                    currentTime,
                    key: const ValueKey(
                      'message-media-video-viewer-current-time',
                    ),
                    style: timeStyle,
                  ),
                  timeline,
                  Text(
                    totalTime,
                    key: const ValueKey(
                      'message-media-video-viewer-total-time',
                    ),
                    style: timeStyle.copyWith(color: Colors.white70),
                  ),
                ],
              ),
            ),
          ),
        );
        return Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (rowWidth > constraints.maxWidth)
              NotificationListener<ScrollNotification>(
                onNotification: (notification) {
                  if (notification.depth == 0) {
                    if (notification is ScrollStartNotification) {
                      onInteractionStart();
                    } else if (notification is ScrollEndNotification) {
                      onInteractionEnd();
                    }
                  }
                  return false;
                },
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: transport,
                ),
              )
            else
              transport,
            const SizedBox(height: Grid.xxs),
            Row(
              children: [
                if (onReply != null)
                  _MediaViewerCircleButton(
                    key: const ValueKey(
                      'message-media-video-viewer-reply-thread',
                    ),
                    icon: LucideIcons.messageSquareReply,
                    tooltip: 'Reply in thread',
                    onPressed: onReply,
                  ),
                const Spacer(),
                _VideoControlSurface(
                  child: SizedBox(
                    width: speedWidth,
                    height: height,
                    child: speed,
                  ),
                ),
                const SizedBox(width: Grid.xxs),
                _VideoControlSurface(
                  child: SizedBox(width: 48, height: height, child: mute),
                ),
              ],
            ),
          ],
        );
      },
    );
  }
}

class _VideoControlSurface extends StatelessWidget {
  const _VideoControlSurface({required this.child});
  final Widget child;

  @override
  Widget build(BuildContext context) {
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      return Theme(
        data: ThemeData.dark(),
        child: ConcentricSheetSurface(
          enabled: true,
          usesGlass: true,
          providesSheetSurface: false,
          padding: EdgeInsets.zero,
          minimumRadius: Radii.dialog,
          contentClipRadius: Radii.dialog,
          color: Colors.black.withValues(alpha: 0.35),
          child: child,
        ),
      );
    }
    return ClipRRect(
      borderRadius: BorderRadius.circular(Radii.dialog),
      child: BackdropFilter(
        filter: ImageFilter.blur(sigmaX: 16, sigmaY: 16),
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: Colors.black.withValues(alpha: 0.35),
            border: Border.all(color: Colors.white.withValues(alpha: 0.10)),
            borderRadius: BorderRadius.circular(Radii.dialog),
          ),
          child: child,
        ),
      ),
    );
  }
}

/// Desktop's narrow vertical playhead, with the slider retaining a full-height
/// touch target independently of this visual marker.
class _VideoTimelineThumb extends SliderComponentShape {
  const _VideoTimelineThumb();

  @override
  Size getPreferredSize(bool isEnabled, bool isDiscrete) => const Size(2, 16);

  @override
  void paint(
    PaintingContext context,
    Offset center, {
    required Animation<double> activationAnimation,
    required Animation<double> enableAnimation,
    required bool isDiscrete,
    required TextPainter labelPainter,
    required RenderBox parentBox,
    required SliderThemeData sliderTheme,
    required TextDirection textDirection,
    required double value,
    required double textScaleFactor,
    required Size sizeWithOverflow,
  }) {
    context.canvas.drawRRect(
      RRect.fromRectAndRadius(
        Rect.fromCenter(center: center, width: 2, height: 16),
        const Radius.circular(1),
      ),
      Paint()..color = Colors.white,
    );
  }
}

String _formatVideoDuration(Duration duration) {
  final totalSeconds = duration.inSeconds.clamp(0, 359999);
  final hours = totalSeconds ~/ 3600;
  final minutes = (totalSeconds % 3600) ~/ 60;
  final seconds = totalSeconds % 60;
  final paddedSeconds = seconds.toString().padLeft(2, '0');
  if (hours > 0) {
    return '$hours:${minutes.toString().padLeft(2, '0')}:$paddedSeconds';
  }
  return '${minutes.toString().padLeft(2, '0')}:$paddedSeconds';
}

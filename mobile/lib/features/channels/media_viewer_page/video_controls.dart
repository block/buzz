part of '../media_viewer_page.dart';

/// Video transport and message actions, kept visually aligned with the image
/// viewer chrome while the native player owns decoding and playback.
class _VideoViewerBottomControls extends StatelessWidget {
  final VideoPlayerController? controller;
  final VoidCallback? onReply;

  const _VideoViewerBottomControls({
    required this.controller,
    required this.onReply,
  });

  @override
  Widget build(BuildContext context) {
    final readyController = controller?.value.isInitialized == true
        ? controller
        : null;
    return Padding(
      padding: const EdgeInsets.fromLTRB(Grid.sm, Grid.xxs, Grid.sm, 0),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (readyController != null)
            _VideoTransportBar(controller: readyController),
          Row(
            children: [
              _MediaViewerCircleButton(
                key: const ValueKey('message-media-video-viewer-reply-thread'),
                icon: LucideIcons.messageSquareReply,
                tooltip: 'Reply in thread',
                onPressed: onReply,
              ),
              const Spacer(),
            ],
          ),
        ],
      ),
    );
  }
}

class _VideoTransportBar extends HookConsumerWidget {
  final VideoPlayerController controller;

  const _VideoTransportBar({required this.controller});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(controller);
    final value = controller.value;
    final durationMs = value.duration.inMilliseconds;
    final positionMs = value.position.inMilliseconds.clamp(0, durationMs);
    final hasDuration = durationMs > 0;

    return Padding(
      padding: const EdgeInsets.only(bottom: Grid.xxs),
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: Colors.black.withValues(alpha: 0.42),
          border: Border.all(color: Colors.white.withValues(alpha: 0.12)),
          borderRadius: BorderRadius.circular(Radii.full),
        ),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
          child: Row(
            children: [
              IconButton(
                key: const ValueKey('message-media-video-viewer-play-pause'),
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
              Text(
                _formatVideoDuration(Duration(milliseconds: positionMs)),
                style: context.textTheme.labelSmall?.copyWith(
                  color: Colors.white,
                  fontFeatures: const [FontFeature.tabularFigures()],
                ),
              ),
              Expanded(
                child: SliderTheme(
                  data: SliderTheme.of(context).copyWith(
                    activeTrackColor: Colors.white,
                    inactiveTrackColor: Colors.white.withValues(alpha: 0.28),
                    thumbColor: Colors.white,
                    overlayColor: Colors.white.withValues(alpha: 0.12),
                    trackHeight: 3,
                    thumbShape: const RoundSliderThumbShape(
                      enabledThumbRadius: 5,
                    ),
                  ),
                  child: Slider(
                    value: hasDuration ? positionMs.toDouble() : 0,
                    min: 0,
                    max: hasDuration ? durationMs.toDouble() : 1,
                    onChanged: hasDuration
                        ? (next) => controller.seekTo(
                            Duration(milliseconds: next.round()),
                          )
                        : null,
                  ),
                ),
              ),
              Text(
                _formatVideoDuration(value.duration),
                style: context.textTheme.labelSmall?.copyWith(
                  color: Colors.white.withValues(alpha: 0.78),
                  fontFeatures: const [FontFeature.tabularFigures()],
                ),
              ),
              const SizedBox(width: Grid.xxs),
            ],
          ),
        ),
      ),
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
  return '$minutes:$paddedSeconds';
}

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import 'voice_note_play_pause_icon.dart';
import 'voice_note_recording.dart';
import 'voice_note_waveform.dart';

/// Displays a recorded or remote voice note with playback controls.
class VoiceNoteAttachment extends HookConsumerWidget {
  const VoiceNoteAttachment.local({
    super.key,
    required String path,
    required this.duration,
    required this.waveform,
    this.onRemove,
  }) : source = path,
       isRemote = false;

  const VoiceNoteAttachment.remote({
    super.key,
    required String url,
    required this.duration,
    this.waveform = const [],
  }) : source = url,
       isRemote = true,
       onRemove = null;

  final String source;
  final bool isRemote;
  final Duration duration;
  final List<double> waveform;
  final VoidCallback? onRemove;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final player = useMemoized(ref.read(voiceNotePlayerFactoryProvider), [
      source,
    ]);
    final playback = useListenable(player);
    final playbackRate = useState(1.0);
    useEffect(() {
      if (isRemote) {
        unawaited(
          player.loadRemote(
            source,
            headers: () =>
                ref.read(mediaGetAuthServiceProvider).headersFor(source),
            fallbackDuration: duration,
          ),
        );
      } else {
        unawaited(player.loadLocal(source, fallbackDuration: duration));
      }
      return player.dispose;
    }, [player, source, isRemote, duration]);

    final state = playback.state;
    final resolvedDuration = state.duration > Duration.zero
        ? state.duration
        : duration;
    final progress = resolvedDuration.inMilliseconds <= 0
        ? 0.0
        : (state.position.inMilliseconds / resolvedDuration.inMilliseconds)
              .clamp(0.0, 1.0);
    final progressAnimation = useAnimationController(initialValue: progress);

    void animateProgressFrom(double fraction) {
      final resolved = fraction.clamp(0.0, 1.0);
      progressAnimation
        ..stop()
        ..value = resolved;
      if (!state.isPlaying || resolvedDuration.inMilliseconds <= 0) return;
      final remainingMilliseconds = math.max(
        1,
        (resolvedDuration.inMilliseconds * (1 - resolved) / playbackRate.value)
            .round(),
      );
      unawaited(
        progressAnimation.animateTo(
          1,
          duration: Duration(milliseconds: remainingMilliseconds),
          curve: Curves.linear,
        ),
      );
    }

    useEffect(
      () {
        animateProgressFrom(
          state.isPlaying ? progressAnimation.value : progress,
        );
        return null;
      },
      [
        state.isPlaying,
        state.isPlaying ? null : state.position.inMilliseconds,
        resolvedDuration.inMilliseconds,
        playbackRate.value,
      ],
    );
    final samples = normalizeVoiceNoteWaveform(
      waveform.isEmpty ? _seededWaveform(source) : waveform,
    );
    final isComposer = !isRemote;
    final radius = isComposer
        ? Radii.dialog + Grid.quarter - Grid.twelve
        : Radii.md;

    final canCancelLoading = state.isLoading && state.canCancelLoading;
    final onPlaybackPressed = state.isLoading && !canCancelLoading
        ? null
        : state.hasError && !isRemote
        ? null
        : () {
            unawaited(HapticFeedback.selectionClick());
            unawaited(player.toggle());
          };
    final playbackControlLabel = state.isLoading
        ? state.isPlaying
              ? 'Pause voice note'
              : canCancelLoading
              ? 'Cancel voice note loading'
              : 'Loading voice note'
        : state.hasError && isRemote
        ? 'Retry voice note'
        : state.isPlaying
        ? 'Pause voice note'
        : 'Play voice note';

    return Container(
      key: ValueKey('voice-note-attachment:$source'),
      constraints: BoxConstraints(
        minWidth: 220,
        maxWidth: isComposer ? double.infinity : 320,
        minHeight: 64,
      ),
      padding: const EdgeInsets.all(Grid.twelve),
      decoration: BoxDecoration(
        color: context.colors.surface,
        borderRadius: BorderRadius.circular(radius),
        border: Border.all(color: context.colors.outlineVariant),
      ),
      child: Row(
        children: [
          SizedBox.square(
            dimension: 40,
            child: Semantics(
              container: true,
              button: true,
              label: playbackControlLabel,
              onTap: onPlaybackPressed,
              excludeSemantics: true,
              child: ExcludeSemantics(
                child: IconButton.filledTonal(
                  key: const ValueKey('voice-note-play-pause'),
                  tooltip: playbackControlLabel,
                  onPressed: onPlaybackPressed,
                  style: IconButton.styleFrom(
                    minimumSize: const Size.square(40),
                    maximumSize: const Size.square(40),
                    padding: EdgeInsets.zero,
                    tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                  ),
                  icon: state.isLoading
                      ? ExcludeSemantics(
                          child: BuzzLoadingIndicator(
                            size: 18,
                            color: context.colors.onSecondaryContainer,
                          ),
                        )
                      : state.hasError && isRemote
                      ? Icon(
                          LucideIcons.refreshCcw,
                          key: const ValueKey('voice-note-retry-icon'),
                          size: 18,
                          color: context.colors.onSecondaryContainer,
                        )
                      : VoiceNotePlayPauseIcon(
                          isPlaying: state.isPlaying,
                          color: context.colors.onSecondaryContainer,
                        ),
                ),
              ),
            ),
          ),
          const SizedBox(width: Grid.xxs),
          Expanded(
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                AnimatedBuilder(
                  animation: progressAnimation,
                  builder: (context, _) => VoiceNoteWaveform(
                    samples: samples,
                    progress: progressAnimation.value,
                    height: 24,
                    onSeek: (fraction) {
                      animateProgressFrom(fraction);
                      unawaited(
                        player.seek(
                          Duration(
                            milliseconds:
                                (resolvedDuration.inMilliseconds * fraction)
                                    .round(),
                          ),
                        ),
                      );
                    },
                  ),
                ),
                Text(
                  key: const ValueKey('voice-note-duration'),
                  state.hasError
                      ? 'Voice note unavailable'
                      : formatVoiceNoteDuration(
                          state.position > Duration.zero
                              ? state.position
                              : resolvedDuration,
                        ),
                  style: context.textTheme.labelSmall?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                ),
              ],
            ),
          ),
          if (isRemote) ...[
            const SizedBox(width: Grid.xxs),
            _VoiceNotePlaybackRateButton(
              key: const ValueKey('voice-note-playback-rate'),
              rate: playbackRate.value,
              onPressed: () {
                unawaited(HapticFeedback.selectionClick());
                final next = nextVoiceNotePlaybackRate(playbackRate.value);
                playbackRate.value = next;
                unawaited(player.setSpeed(next));
              },
            ),
          ] else if (onRemove != null) ...[
            const SizedBox(width: Grid.xxs),
            SizedBox.square(
              dimension: 40,
              child: IconButton(
                key: const ValueKey('composer-voice-note-remove'),
                tooltip: 'Remove voice note',
                onPressed: onRemove,
                style: IconButton.styleFrom(
                  minimumSize: const Size.square(40),
                  maximumSize: const Size.square(40),
                  padding: EdgeInsets.zero,
                  tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                ),
                icon: const Icon(LucideIcons.x, size: 18),
              ),
            ),
          ],
        ],
      ),
    );
  }
}

class _VoiceNotePlaybackRateButton extends StatelessWidget {
  const _VoiceNotePlaybackRateButton({
    super.key,
    required this.rate,
    required this.onPressed,
  });

  final double rate;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => Semantics(
    button: true,
    label: 'Playback speed ${formatVoiceNotePlaybackRate(rate)}',
    hint:
        'Double tap to change to ${formatVoiceNotePlaybackRate(nextVoiceNotePlaybackRate(rate))}.',
    child: Tooltip(
      message: 'Playback speed',
      child: Material(
        color: context.colors.primary,
        borderRadius: BorderRadius.circular(Radii.full),
        child: InkWell(
          onTap: onPressed,
          borderRadius: BorderRadius.circular(Radii.full),
          child: Padding(
            padding: const EdgeInsets.symmetric(
              horizontal: Grid.xxs,
              vertical: Grid.half + Grid.quarter,
            ),
            child: Stack(
              alignment: Alignment.center,
              children: [
                ExcludeSemantics(
                  child: Opacity(
                    opacity: 0,
                    child: Text('1.5×', style: _rateStyle(context)),
                  ),
                ),
                Positioned.fill(
                  child: Center(
                    child: Text(
                      formatVoiceNotePlaybackRate(rate),
                      key: const ValueKey('voice-note-playback-rate-value'),
                      textAlign: TextAlign.center,
                      style: _rateStyle(context),
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

  TextStyle? _rateStyle(BuildContext context) =>
      context.textTheme.labelSmall?.copyWith(
        color: context.colors.onPrimary,
        fontWeight: FontWeight.w700,
        fontFeatures: const [FontFeature.tabularFigures()],
      );
}

List<double> _seededWaveform(String seed) {
  var value = seed.hashCode & 0x7fffffff;
  return List.generate(48, (_) {
    value = (1103515245 * value + 12345) & 0x7fffffff;
    return 0.12 + ((value % 760) / 1000);
  });
}

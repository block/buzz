import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import '../../shared/recording_haptics.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/navigation_button_surface.dart';
import '../../shared/widgets/concentric_sheet_surface.dart';
import '../../shared/widgets/agent_navigation_overlay.dart';
import 'bestie_transcription_loading_indicator.dart';
import 'voice_note_recording.dart';
import 'voice_note_waveform.dart';

/// Recording presentation from PR #8665, with transcription behind its callbacks.
class BestieTranscriptionSurface extends HookWidget {
  const BestieTranscriptionSurface({
    super.key,
    required this.enabled,
    required this.recording,
    this.preparing = false,
    this.processingLabel = 'Processing transcript',
    required this.finalizing,
    required this.sending,
    required this.restoring,
    required this.restoreFailed,
    required this.hasPending,
    required this.elapsed,
    required this.samples,
    required this.error,
    required this.cancelled,
    required this.start,
    required this.finish,
    required this.cancel,
    required this.retry,
    required this.restore,
    required this.discardPending,
    required this.child,
  });
  final bool preparing;
  final String processingLabel;
  final bool enabled,
      recording,
      finalizing,
      sending,
      restoring,
      restoreFailed,
      hasPending;
  final Duration elapsed;
  final List<double> samples;
  final String? error;
  final ValueNotifier<bool> cancelled;
  final Future<void> Function() start,
      finish,
      cancel,
      retry,
      restore,
      discardPending;
  final Widget child;
  @override
  Widget build(BuildContext context) {
    final alive = useRef(true);
    final cancelHapticPrepared = useRef(false);
    useEffect(
      () => () {
        alive.value = false;
      },
      const [],
    );
    final diameter = NavigationGlassGroup.diameterOf(context);
    final active =
        preparing ||
        recording ||
        (!finalizing && !sending && (hasPending || restoreFailed));
    final recordingCallback = AgentNavigationOverlay.recordingCallbackOf(
      context,
    );
    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (alive.value && context.mounted) {
          recordingCallback?.call(recording);
        }
      });
      return null;
    }, [recording, recordingCallback]);
    final latestRecordingCallback = useRef(recordingCallback);
    latestRecordingCallback.value = recordingCallback;
    useEffect(() {
      return () {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          latestRecordingCallback.value?.call(false);
        });
      };
    }, const []);
    final overlay = AgentNavigationOverlay.maybeOf(context);
    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (alive.value && context.mounted) overlay?.value = recording;
      });
      return null;
    }, [recording, overlay]);
    final canRecord =
        enabled && !active && !restoring && !finalizing && !sending;
    final recoveryPortal = useMemoized(OverlayPortalController.new);
    final recoveryLink = useMemoized(LayerLink.new);
    final recoveryAnchor = useMemoized(GlobalKey.new);
    final recovering = active && !recording && !preparing;
    final showRecovery =
        recovering && !AgentNavigationOverlay.menuOpenOf(context);
    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted) return;
        if (showRecovery) {
          recoveryPortal.show();
        } else {
          recoveryPortal.hide();
        }
      });
      return null;
    }, [showRecovery]);
    final pill = Semantics(
      key: const ValueKey('default-agent-voice-status'),
      liveRegion: preparing || sending || finalizing,
      value: preparing
          ? 'Preparing speech. Wait before speaking.'
          : recording
          ? cancelled.value
                ? 'Release to cancel'
                : 'Recording ${formatVoiceNoteDuration(elapsed)}'
          : sending
          ? processingLabel
          : finalizing
          ? 'Saving recording'
          : restoring
          ? 'Restoring transcript'
          : restoreFailed
          ? 'Could not restore transcript'
          : hasPending
          ? 'Transcript awaiting delivery'
          : null,
      hint: preparing
          ? 'Release to cancel preparation.'
          : canRecord
          ? 'Hold to record a transcript. Release to send. Slide up to cancel.'
          : recording
          ? 'Release to send. Slide up to cancel.'
          : null,
      customSemanticsActions: canRecord
          ? {
              const CustomSemanticsAction(label: 'Record transcript'): () =>
                  unawaited(start()),
            }
          : recording || preparing
          ? {
              const CustomSemanticsAction(label: 'Discard transcript'): () =>
                  unawaited(cancel()),
            }
          : null,
      child: Listener(
        onPointerDown: (_) {
          if (canRecord) unawaited(prepareRecordingHaptic());
        },
        onPointerCancel: (_) => unawaited(cancel()),
        child: GestureDetector(
          excludeFromSemantics: true,
          behavior: HitTestBehavior.opaque,
          onLongPressStart: enabled ? (_) => unawaited(start()) : null,
          onLongPressMoveUpdate: (details) {
            if (recording) {
              final willCancel = details.offsetFromOrigin.dy < -64;
              if (details.offsetFromOrigin.dy < -24 &&
                  !cancelHapticPrepared.value) {
                cancelHapticPrepared.value = true;
                unawaited(prepareRecordingHaptic(cancel: true));
              } else if (details.offsetFromOrigin.dy >= -24) {
                cancelHapticPrepared.value = false;
              }
              if (willCancel && !cancelled.value) {
                unawaited(recordingHaptic(cancel: true));
              }
              cancelled.value = willCancel;
            }
          },
          onLongPressEnd: (_) => unawaited(finish()),
          onLongPressCancel: () => unawaited(cancel()),
          child: AnimatedContainer(
            key: const ValueKey('default-agent-voice-pill'),
            duration: MediaQuery.disableAnimationsOf(context)
                ? Duration.zero
                : const Duration(milliseconds: 220),
            curve: Curves.easeInOutCubic,
            width: active ? 224 + Grid.xs * 2 : diameter,
            height: diameter,
            // Keep one native glass view mounted while the finger is down.
            // Replacing its UIKit subtree cancels the active iOS touch.
            child: Stack(
              clipBehavior: Clip.none,
              children: [
                Positioned.fill(
                  child: NavigationButtonSurface(
                    child: Stack(
                      fit: StackFit.expand,
                      children: [
                        Positioned.fill(
                          child: IgnorePointer(
                            child: DecoratedBox(
                              key: const ValueKey(
                                'default-agent-voice-surface',
                              ),
                              decoration: BoxDecoration(
                                color: recording
                                    ? context.colors.primary
                                    : active &&
                                          NavigationGlassGroup.maybeOf(context)
                                    ? context.controlSurface
                                    : Colors.transparent,
                                borderRadius: BorderRadius.circular(
                                  Radii.navigationCapsule,
                                ),
                              ),
                            ),
                          ),
                        ),
                        Positioned.fill(
                          child: !active
                              ? const SizedBox.shrink()
                              : Padding(
                                  padding: const EdgeInsets.symmetric(
                                    horizontal: Grid.xxs,
                                  ),
                                  child: ClipRect(
                                    child: OverflowBox(
                                      alignment: Alignment.centerRight,
                                      minWidth:
                                          224 + Grid.xs * 2 - Grid.xxs * 2,
                                      maxWidth:
                                          224 + Grid.xs * 2 - Grid.xxs * 2,
                                      child: Row(
                                        children: [
                                          if (!recording &&
                                              !sending &&
                                              (hasPending || restoreFailed))
                                            IconButton(
                                              tooltip: 'Discard transcript',
                                              onPressed: () =>
                                                  unawaited(discardPending()),
                                              icon: const Icon(Icons.close),
                                            ),
                                          Expanded(
                                            child: sending || finalizing
                                                ? const Center(
                                                    child: BestieTranscriptionLoadingIndicator(
                                                      semanticLabel:
                                                          'Sending transcript',
                                                      size: 28,
                                                    ),
                                                  )
                                                : preparing
                                                ? const Text(
                                                    'Preparing speech…',
                                                    textAlign: TextAlign.center,
                                                  )
                                                : restoreFailed
                                                ? TextButton(
                                                    onPressed: restoring
                                                        ? null
                                                        : () => unawaited(
                                                            restore(),
                                                          ),
                                                    child: Text(
                                                      error!,
                                                      maxLines: 2,
                                                    ),
                                                  )
                                                : hasPending
                                                ? TextButton(
                                                    onPressed: () =>
                                                        unawaited(retry()),
                                                    child: Text(
                                                      'Retry transcript',
                                                    ),
                                                  )
                                                : VoiceNoteWaveform(
                                                    color: context
                                                        .colors
                                                        .onPrimary,
                                                    samples: [
                                                      ...List.filled(
                                                        41 - samples.length,
                                                        0.0,
                                                      ),
                                                      ...samples,
                                                    ],
                                                    progress: 1,
                                                    fadeEdges: true,
                                                    height: 32,
                                                  ),
                                          ),
                                          if (recording)
                                            const SizedBox(width: 48),
                                        ],
                                      ),
                                    ),
                                  ),
                                ),
                        ),
                        // The avatar can itself own an iOS video platform view.
                        // Keep it mounted at the same location throughout the hold.
                        Positioned(
                          right: 0,
                          top: 0,
                          width: diameter,
                          height: diameter,
                          child: ExcludeSemantics(
                            excluding: active,
                            child: IgnorePointer(
                              ignoring: active,
                              child: Opacity(
                                opacity: !active || recording ? 1 : 0,
                                child: TooltipTheme(
                                  data: TooltipTheme.of(context).copyWith(
                                    triggerMode: TooltipTriggerMode.manual,
                                  ),
                                  child: finalizing || sending
                                      ? Center(
                                          child:
                                              BestieTranscriptionLoadingIndicator(
                                                semanticLabel: processingLabel,
                                                size: 28,
                                              ),
                                        )
                                      : recovering
                                      ? const SizedBox.shrink()
                                      : child,
                                ),
                              ),
                            ),
                          ),
                        ),
                        if (recording)
                          Positioned(
                            right: 0,
                            top: 0,
                            width: diameter,
                            height: diameter,
                            child: IconButton(
                              tooltip: 'Send transcript',
                              onPressed: () => unawaited(finish()),
                              icon: const SizedBox.square(dimension: 48),
                            ),
                          ),
                      ],
                    ),
                  ),
                ),
                if (recording)
                  Positioned(
                    bottom: 56 + Grid.xs,
                    width: 224 + Grid.xs * 2,
                    right: 0,
                    child: Center(
                      child: Semantics(
                        label: cancelled.value
                            ? 'Release to cancel'
                            : 'Slide up to cancel',
                        child: IgnorePointer(
                          child: AnimatedScale(
                            key: const ValueKey('voice-cancel-target'),
                            scale: cancelled.value ? 1.15 : 1,
                            duration: MediaQuery.disableAnimationsOf(context)
                                ? Duration.zero
                                : const Duration(milliseconds: 220),
                            curve: Curves.easeOutBack,
                            child: ConcentricSheetSurface(
                              enabled: true,
                              usesGlass: true,
                              providesSheetSurface: false,
                              padding: EdgeInsets.zero,
                              minimumRadius: 24,
                              contentClipRadius: 24,
                              color: cancelled.value
                                  ? context.colors.error
                                  : context.colors.surfaceContainerHighest,
                              glassTintColor: cancelled.value
                                  ? context.colors.error
                                  : null,
                              child: AnimatedContainer(
                                duration:
                                    MediaQuery.disableAnimationsOf(context)
                                    ? Duration.zero
                                    : const Duration(milliseconds: 140),
                                width: 48,
                                height: 48,
                                decoration: BoxDecoration(
                                  shape: BoxShape.circle,
                                  color:
                                      defaultTargetPlatform ==
                                          TargetPlatform.iOS
                                      ? Colors.transparent
                                      : cancelled.value
                                      ? context.colors.error
                                      : context.colors.surfaceContainerHighest,
                                ),
                                child: Icon(
                                  Icons.close,
                                  color: cancelled.value
                                      ? context.colors.onError
                                      : context.colors.onSurface,
                                ),
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
    return OverlayPortal(
      controller: recoveryPortal,
      overlayChildBuilder: (overlayContext) {
        final box =
            recoveryAnchor.currentContext?.findRenderObject() as RenderBox?;
        final origin = box?.localToGlobal(Offset.zero) ?? Offset.zero;
        final right = origin.dx + (box?.size.width ?? 56);
        final screen = MediaQuery.sizeOf(overlayContext);
        final left = (right - 256).clamp(8.0, screen.width - 264);
        final below = origin.dy < 72;
        return Positioned.fill(
          child: Align(
            alignment: Alignment.topLeft,
            child: CompositedTransformFollower(
              link: recoveryLink,
              showWhenUnlinked: false,
              targetAnchor: below ? Alignment.bottomRight : Alignment.topRight,
              followerAnchor: below
                  ? Alignment.topRight
                  : Alignment.bottomRight,
              offset: Offset(left - (right - 256), below ? 8 : -8),
              child: SizedBox(width: 256, height: 56, child: pill),
            ),
          ),
        );
      },
      child: CompositedTransformTarget(
        key: recoveryAnchor,
        link: recoveryLink,
        child: recovering ? child : pill,
      ),
    );
  }
}

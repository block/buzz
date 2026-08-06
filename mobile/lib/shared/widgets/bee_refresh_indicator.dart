import 'dart:async' show Timer, unawaited;
import 'dart:math' show cos, min, pi, sin;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme.dart';
import 'flapping_bee.dart';

/// Replaces the standard pull-to-refresh spinner with Buzz's loading bee.
///
/// Flutter continues to own the gesture, refresh lifecycle, and accessibility
/// semantics. This widget maps those states into the elastic pull, retained
/// loading gap, and bee animation.
class BeeRefreshIndicator extends HookConsumerWidget {
  /// Called when the user completes a pull, to load fresh data.
  ///
  /// The bee keeps flapping until this future settles, so it should complete
  /// only once the refresh is done.
  final Future<void> Function() onRefresh;

  /// The scrollable this indicator wraps.
  ///
  /// It must scroll vertically; the indicator reads its scroll notifications
  /// to couple the bee to the user's finger.
  final Widget child;

  /// The vertical offset of the scrollable's top edge, such as a pinned header.
  final double edgeOffset;

  const BeeRefreshIndicator({
    required this.onRefresh,
    required this.child,
    this.edgeOffset = 0,
    super.key,
  });

  static const _beeWidth = 60.0;
  static const _beeHeight = _beeWidth * 309 / 466;
  static const _triggerDistance = 100.0;
  static const _loadingGap = 72.0;
  static const _beeVerticalAlignment = 0.75;
  static const _beeInitialScale = 0.6;
  static const _beeRevealStartProgress = 0.18;
  static const _pupilStartBeyondArmDistance = 96.0;
  static const _pupilFullBeforeEmojiDistance = 8.0;
  static const _eyeEmojiSwapViewportFraction = 0.26;
  static const _eyeEmojiMinSwapBeyondArmDistance = 220.0;
  static const _eyeEmojiMaxSwapBeyondArmDistance = 280.0;
  static const _pupilMinBeyondArmDuration = Duration(milliseconds: 300);
  static const _eyeEmojiMinBeyondArmDuration = Duration(milliseconds: 700);
  static const _eyeShakePeriod = Duration(milliseconds: 140);
  static const _settleDuration = Duration(milliseconds: 180);

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final status = useState<RefreshIndicatorStatus?>(null);
    final pullProgress = useState(0.0);
    final pullDistance = useState(0.0);
    final pullBeyondArmDistance = useState(0.0);
    final pullBeyondArmDuration = useState(Duration.zero);
    final activePointers = useRef(<int>{});
    final lastPointerTime = useRef(Duration.zero);
    final armedAt = useRef<Duration?>(null);
    final didTriggerArmHaptic = useRef(false);
    final didTriggerEmojiHaptic = useRef(false);
    final eyeShakeHapticTimer = useRef<Timer?>(null);
    final completionController = useAnimationController(
      duration: _settleDuration,
    );
    final gapController = useAnimationController(
      duration: _settleDuration,
      reverseDuration: _settleDuration,
    );
    final flapController = useAnimationController(
      duration: const Duration(milliseconds: 480),
    );
    final eyeShakeController = useAnimationController(
      duration: _eyeShakePeriod,
    );
    final completionProgress = useAnimation(completionController);
    final gapProgress = useAnimation(gapController);
    final flapProgress = useAnimation(flapController);
    final eyeShakeProgress = useAnimation(eyeShakeController);
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    useEffect(
      () =>
          () => eyeShakeHapticTimer.value?.cancel(),
      const [],
    );
    final eyeEmojiSwapDistance =
        (MediaQuery.sizeOf(context).height * _eyeEmojiSwapViewportFraction)
            .clamp(
              _eyeEmojiMinSwapBeyondArmDistance,
              _eyeEmojiMaxSwapBeyondArmDistance,
            )
            .toDouble();

    void stopEyeShake() {
      eyeShakeController
        ..stop()
        ..reset();
      eyeShakeHapticTimer.value?.cancel();
      eyeShakeHapticTimer.value = null;
    }

    void startEyeShake() {
      stopEyeShake();
      if (reducedMotion) return;

      eyeShakeController.repeat();
      eyeShakeHapticTimer.value = Timer.periodic(
        _eyeShakePeriod,
        (_) => unawaited(HapticFeedback.selectionClick()),
      );
    }

    void beginExpressionTracking() {
      if (activePointers.value.isEmpty || armedAt.value != null) return;
      pullBeyondArmDistance.value = 0;
      pullBeyondArmDuration.value = Duration.zero;
      armedAt.value = lastPointerTime.value;
      if (!didTriggerArmHaptic.value) {
        didTriggerArmHaptic.value = true;
        unawaited(HapticFeedback.mediumImpact());
      }
    }

    void updateStatus(RefreshIndicatorStatus? nextStatus) {
      status.value = nextStatus;

      if (nextStatus == RefreshIndicatorStatus.drag) {
        completionController.reset();
        gapController.reset();
        flapController.stop();
        if (!didTriggerArmHaptic.value) {
          pullBeyondArmDistance.value = 0;
          pullBeyondArmDuration.value = Duration.zero;
          armedAt.value = null;
        }
      } else if (nextStatus == RefreshIndicatorStatus.armed) {
        pullProgress.value = 1;
        beginExpressionTracking();
      } else if (nextStatus == RefreshIndicatorStatus.snap ||
          nextStatus == RefreshIndicatorStatus.refresh) {
        stopEyeShake();
        pullProgress.value = 1;
        pullBeyondArmDistance.value = 0;
        pullBeyondArmDuration.value = Duration.zero;
        armedAt.value = null;
        if (reducedMotion) {
          gapController.value = 1;
        } else {
          gapController.animateTo(1, curve: Curves.easeOutCubic);
        }
        if (!reducedMotion) flapController.repeat();
      } else if (nextStatus == RefreshIndicatorStatus.done) {
        stopEyeShake();
        if (reducedMotion) {
          gapController.value = 0;
          pullProgress.value = 0;
          pullDistance.value = 0;
          pullBeyondArmDistance.value = 0;
          pullBeyondArmDuration.value = Duration.zero;
          status.value = null;
        } else {
          flapController.repeat();
          gapController
              .animateTo(1, curve: Curves.easeOutCubic)
              .whenCompleteOrCancel(() {
                if (!gapController.isCompleted || status.value == null) return;
                gapController.animateBack(0, curve: Curves.easeInOutCubic);
                completionController.forward(from: 0).whenCompleteOrCancel(() {
                  if (!completionController.isCompleted) return;
                  flapController.stop();
                  pullProgress.value = 0;
                  pullDistance.value = 0;
                  pullBeyondArmDistance.value = 0;
                  pullBeyondArmDuration.value = Duration.zero;
                  status.value = null;
                });
              });
        }
      } else if (nextStatus == RefreshIndicatorStatus.canceled ||
          nextStatus == null) {
        stopEyeShake();
        pullProgress.value = 0;
        pullDistance.value = 0;
        pullBeyondArmDistance.value = 0;
        pullBeyondArmDuration.value = Duration.zero;
        armedAt.value = null;
        if (!gapController.isAnimating) gapController.reset();
        flapController.stop();
      }
    }

    bool trackPull(ScrollNotification notification) {
      if (notification.metrics.axis != Axis.vertical) return false;

      if (notification is! ScrollStartNotification &&
          notification.metrics.extentBefore == 0) {
        // BouncingScrollPhysics reports a live negative scroll position while
        // the user is pulling. Reading it keeps the bee coupled to the finger.
        final elasticPull =
            (notification.metrics.minScrollExtent - notification.metrics.pixels)
                .clamp(0.0, double.infinity)
                .toDouble();
        if (elasticPull > 0 || pullDistance.value > 0) {
          pullDistance.value = elasticPull;
          final nextProgress = (elasticPull / _triggerDistance).clamp(0.0, 1.0);
          pullProgress.value = nextProgress;
          if (nextProgress >= 1) beginExpressionTracking();
        } else if (notification case OverscrollNotification()) {
          // Clamping physics does not expose a negative position, so build the
          // same progress from its overscroll deltas.
          final nextDistance =
              pullDistance.value + notification.overscroll.abs();
          pullDistance.value = nextDistance;
          pullProgress.value = (nextDistance / _triggerDistance).clamp(
            0.0,
            1.0,
          );
          if (pullProgress.value >= 1) beginExpressionTracking();
        }
      }
      return false;
    }

    void startPointer(PointerDownEvent event) {
      final isNewGesture = activePointers.value.isEmpty;
      activePointers.value.add(event.pointer);
      if (!isNewGesture) return;

      lastPointerTime.value = event.timeStamp;
      armedAt.value = null;
      didTriggerArmHaptic.value = false;
      didTriggerEmojiHaptic.value = false;
      stopEyeShake();
      pullProgress.value = 0;
      pullDistance.value = 0;
      pullBeyondArmDistance.value = 0;
      pullBeyondArmDuration.value = Duration.zero;
    }

    void trackPointer(PointerMoveEvent event) {
      if (!activePointers.value.contains(event.pointer)) return;
      lastPointerTime.value = event.timeStamp;
      if (armedAt.value == null) return;

      pullBeyondArmDistance.value =
          (pullBeyondArmDistance.value + event.delta.dy)
              .clamp(0.0, double.infinity)
              .toDouble();
      if (armedAt.value case final armedTime?) {
        pullBeyondArmDuration.value = event.timeStamp - armedTime;
      }
      if (!didTriggerEmojiHaptic.value &&
          pullBeyondArmDuration.value >= _eyeEmojiMinBeyondArmDuration &&
          pullBeyondArmDistance.value >= eyeEmojiSwapDistance) {
        didTriggerEmojiHaptic.value = true;
        unawaited(HapticFeedback.heavyImpact());
        startEyeShake();
      }
    }

    void finishPointer(PointerEvent event) {
      if (!activePointers.value.remove(event.pointer)) return;
      if (activePointers.value.isNotEmpty) return;

      stopEyeShake();
      armedAt.value = null;
      pullBeyondArmDistance.value = 0;
      pullBeyondArmDuration.value = Duration.zero;
    }

    final isLoading = switch (status.value) {
      RefreshIndicatorStatus.snap ||
      RefreshIndicatorStatus.refresh ||
      RefreshIndicatorStatus.done => true,
      _ => false,
    };
    final dragRevealProgress =
        ((pullProgress.value - _beeRevealStartProgress) /
                (1 - _beeRevealStartProgress))
            .clamp(0.0, 1.0);
    final isVisible =
        status.value != null &&
        (isLoading || dragRevealProgress > 0 || completionProgress > 0);
    final retainedGap = _loadingGap * gapProgress;
    final visibleGap = pullDistance.value + retainedGap;
    final top = edgeOffset + (visibleGap - _beeHeight) * _beeVerticalAlignment;
    final opacity = isLoading ? 1 - completionProgress : dragRevealProgress;
    final beeScale = isLoading
        ? 1.0
        : _beeInitialScale + (1 - _beeInitialScale) * dragRevealProgress;
    final flapAmount = reducedMotion
        ? 0.0
        : isLoading
        ? 0.5 - (0.5 * cos(flapProgress * 4 * pi))
        : pullProgress.value * 0.18;
    final pupilFullDistance =
        eyeEmojiSwapDistance - _pupilFullBeforeEmojiDistance;
    final pupilDistanceProgress =
        ((pullBeyondArmDistance.value - _pupilStartBeyondArmDistance) /
                (pupilFullDistance - _pupilStartBeyondArmDistance))
            .clamp(0.0, 1.0);
    final pupilGrowthDuration =
        _eyeEmojiMinBeyondArmDuration - _pupilMinBeyondArmDuration;
    final pupilProgress =
        pullBeyondArmDuration.value >= _pupilMinBeyondArmDuration
        ? min(
            pupilDistanceProgress,
            ((pullBeyondArmDuration.value - _pupilMinBeyondArmDuration)
                        .inMicroseconds /
                    pupilGrowthDuration.inMicroseconds)
                .clamp(0.0, 1.0),
          ).toDouble()
        : 0.0;
    final showEyeEmoji = !isLoading && didTriggerEmojiHaptic.value;
    final eyeShakeOffset = showEyeEmoji && !reducedMotion
        ? sin(eyeShakeProgress * 2 * pi) * 0.75
        : 0.0;
    final scrollBehavior = ScrollConfiguration.of(context).copyWith(
      overscroll: false,
      physics: const BouncingScrollPhysics(
        parent: AlwaysScrollableScrollPhysics(),
      ),
    );

    return Stack(
      clipBehavior: Clip.none,
      children: [
        RefreshIndicator.noSpinner(
          onRefresh: onRefresh,
          onStatusChange: updateStatus,
          semanticsLabel: 'Pull to refresh',
          child: NotificationListener<ScrollNotification>(
            onNotification: trackPull,
            child: Listener(
              behavior: HitTestBehavior.translucent,
              onPointerDown: startPointer,
              onPointerMove: trackPointer,
              onPointerUp: finishPointer,
              onPointerCancel: finishPointer,
              child: ScrollConfiguration(
                behavior: scrollBehavior,
                child: Transform.translate(
                  key: const ValueKey('bee-refresh-retained-gap'),
                  offset: Offset(0, retainedGap),
                  child: child,
                ),
              ),
            ),
          ),
        ),
        if (isVisible)
          Positioned(
            top: edgeOffset,
            left: 0,
            right: 0,
            bottom: 0,
            child: ClipRect(
              child: Align(
                alignment: Alignment.topCenter,
                child: Transform.translate(
                  offset: Offset(0, top - edgeOffset),
                  child: IgnorePointer(
                    child: Opacity(
                      key: const ValueKey('bee-refresh-opacity'),
                      opacity: opacity.clamp(0.0, 1.0),
                      child: Transform.scale(
                        key: const ValueKey('bee-refresh-scale'),
                        scale: beeScale,
                        child: SizedBox(
                          width: _beeWidth,
                          height: _beeHeight,
                          child: Stack(
                            clipBehavior: Clip.none,
                            alignment: Alignment.topCenter,
                            children: [
                              Positioned.fill(
                                child: FlappingBee(
                                  width: _beeWidth,
                                  color: context.colors.primary,
                                  flapAmount: flapAmount,
                                  eyeProgress:
                                      !isLoading &&
                                          !showEyeEmoji &&
                                          pupilProgress > 0
                                      ? pupilProgress
                                      : null,
                                ),
                              ),
                              if (showEyeEmoji)
                                Positioned(
                                  top: 2,
                                  left: 0,
                                  right: 0,
                                  child: ExcludeSemantics(
                                    child: Transform.translate(
                                      key: const ValueKey(
                                        'bee-refresh-eyes-emoji-offset',
                                      ),
                                      offset: const Offset(2, 0),
                                      child: Transform.translate(
                                        key: const ValueKey(
                                          'bee-refresh-eyes-emoji-shake',
                                        ),
                                        offset: Offset(eyeShakeOffset, 0),
                                        child: const Text(
                                          '👀',
                                          key: ValueKey(
                                            'bee-refresh-eyes-emoji',
                                          ),
                                          textAlign: TextAlign.center,
                                          style: TextStyle(
                                            fontSize: 18,
                                            height: 1,
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
                  ),
                ),
              ),
            ),
          ),
      ],
    );
  }
}

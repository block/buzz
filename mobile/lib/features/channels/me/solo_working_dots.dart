import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Desktop's three quiet opacity pulses. The containing row owns semantics.
///
/// Every visible row steps from one shared clock at about 12 fps instead of
/// animating at display rate. On Home each frame costs the app, backboardd
/// and the kernel far more than painting the dots does, so the frame rate is
/// the cost; sharing the clock also keeps rows in phase in the same frames.
class SoloWorkingDots extends HookWidget {
  const SoloWorkingDots({super.key, required this.color});

  final Color color;

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final animate = TickerMode.valuesOf(context).enabled && !reducedMotion;
    // A per-row revision, not the shared step: a row that rejoins while
    // another row kept the clock running must still rebuild on its next tick.
    final revision = useState(0);
    useEffect(() {
      if (!animate) return null;
      void onStep() => revision.value++;
      _PulseClock.instance.add(onStep);
      return () => _PulseClock.instance.remove(onStep);
    }, [animate]);
    final phase = _PulseClock.instance.step / _PulseClock.stepsPerCycle;
    return ExcludeSemantics(
      child: RepaintBoundary(
        child: SizedBox(
          height: 20,
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              for (var index = 0; index < 3; index++) ...[
                if (index > 0) const SizedBox(width: 2),
                Opacity(
                  opacity: reducedMotion
                      ? 1
                      : 0.35 +
                            0.65 *
                                (1 -
                                    math.cos(
                                      2 * math.pi * (phase - index / 7),
                                    )) /
                                2,
                  child: DecoratedBox(
                    decoration: BoxDecoration(
                      color: color,
                      shape: BoxShape.circle,
                    ),
                    child: const SizedBox.square(dimension: 4),
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

/// Steps the 1400 ms pulse in 17 steps (about 12 fps) while any dots are
/// visible and the app is in the foreground.
class _PulseClock {
  static final instance = _PulseClock();
  static const stepsPerCycle = 17;
  static const _period = Duration(microseconds: 1400000 ~/ stepsPerCycle);

  final _listeners = <VoidCallback>{};
  int step = 0;
  Timer? _timer;
  AppLifecycleListener? _lifecycle;

  void add(VoidCallback listener) {
    _listeners.add(listener);
    _sync();
  }

  void remove(VoidCallback listener) {
    _listeners.remove(listener);
    _sync();
  }

  void _sync() {
    if (_listeners.isEmpty) {
      _lifecycle?.dispose();
      _lifecycle = null;
    } else {
      _lifecycle ??= AppLifecycleListener(onStateChange: (_) => _sync());
    }
    final state = WidgetsBinding.instance.lifecycleState;
    final foreground =
        state == null ||
        state == AppLifecycleState.resumed ||
        state == AppLifecycleState.inactive;
    if (_listeners.isNotEmpty && foreground) {
      _timer ??= Timer.periodic(_period, (_) {
        step = (step + 1) % stepsPerCycle;
        for (final listener in List.of(_listeners)) {
          listener();
        }
      });
    } else {
      _timer?.cancel();
      _timer = null;
    }
  }
}

/// Whether the shared pulse timer exists, for tests.
@visibleForTesting
bool get debugSoloWorkingDotsClockRunning =>
    _PulseClock.instance._timer != null;

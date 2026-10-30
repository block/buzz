part of '../pairing_page.dart';

class _PairingErrorShake extends HookConsumerWidget {
  const _PairingErrorShake({required this.revision, required this.child});
  final int revision;
  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final shake = useAnimationController(
      duration: const Duration(milliseconds: 280),
    );
    useEffect(() {
      if (reducedMotion || revision == 0) {
        shake.reset();
      } else if (revision > 0) {
        shake.forward(from: 0);
      }
      return null;
    }, [revision, reducedMotion]);
    final shakeOffset = useMemoized(
      () => TweenSequence<double>([
        // Transitions.dev stops: 0%, 28.57%, 57.14%, 78.57%, 100%.
        // Each leg uses the same easing curve independently.
        for (final (begin, end, weight) in [
          (0.0, 6.0, 80.0),
          (6.0, -6.0, 80.0),
          (-6.0, 4.0, 60.0),
          (4.0, 0.0, 60.0),
        ])
          TweenSequenceItem(
            tween: Tween(
              begin: begin,
              end: end,
            ).chain(CurveTween(curve: const Cubic(0.22, 1, 0.36, 1))),
            weight: weight,
          ),
      ]).animate(shake),
      [shake],
    );
    return AnimatedBuilder(
      animation: shakeOffset,
      builder: (context, child) => Transform.translate(
        key: const Key('pairing-error-shake'),
        offset: Offset(reducedMotion ? 0 : shakeOffset.value, 0),
        child: child,
      ),
      child: child,
    );
  }
}

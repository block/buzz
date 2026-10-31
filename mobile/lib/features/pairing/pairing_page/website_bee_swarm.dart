part of '../pairing_page.dart';

class _WebsiteBeeSwarm extends HookWidget {
  final Size size;
  final ObjectRef<Offset?> pointer;

  const _WebsiteBeeSwarm({required this.size, required this.pointer});

  @override
  Widget build(BuildContext context) {
    final motion = useMemoized(() => _WebsiteBeeMotion(size));
    if (motion.size != size) {
      motion.resize(size);
      pointer.value = null;
    }
    final clock = useAnimationController(duration: const Duration(days: 1));
    final lifecycle = useAppLifecycleState();
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final playing =
        !reducedMotion &&
        TickerMode.valuesOf(context).enabled &&
        (lifecycle == null || lifecycle == AppLifecycleState.resumed);
    useEffect(() {
      if (!playing) return null;
      var previous = Duration.zero;
      var remainder = 0.0;
      void tick() {
        final now = clock.lastElapsedDuration ?? Duration.zero;
        // Fixed 60 Hz reproduces the site's per-frame damping on 60/120 Hz
        // phones. Cap catch-up after stalls; never simulate a background gap.
        remainder += ((now - previous).inMicroseconds / 1e6).clamp(0, 0.05);
        previous = now;
        while (remainder >= 1 / 60) {
          motion.advance(1 / 60, pointer.value);
          remainder -= 1 / 60;
        }
      }

      clock.addListener(tick);
      clock.repeat();
      return () {
        clock.removeListener(tick);
        clock.stop();
      };
    }, [clock, motion, playing]);

    return ClipRect(
      key: const Key('welcome-bee-flight-area'),
      child: Stack(
        children: [
          for (var index = 0; index < motion.bees.length; index++)
            AnimatedBuilder(
              animation: clock,
              child: SvgPicture.asset(
                'assets/images/welcome-bee.svg',
                width: 46,
                height: 31,
                colorFilter: ColorFilter.mode(
                  index.isOdd
                      ? const Color(0xFFD7E7F5)
                      : const Color(0xFF231E1E),
                  BlendMode.srcIn,
                ),
              ),
              builder: (context, child) {
                final bee = motion.bees[index];
                final progress = (motion.elapsed / 1.05).clamp(0.0, 1.0);
                final entrance = reducedMotion
                    ? 1.0
                    : 1 - math.pow(1 - progress, 3).toDouble();
                final position = Offset.lerp(
                  bee.entrance,
                  bee.position,
                  entrance,
                )!;
                return Transform.translate(
                  key: ValueKey('welcome-bee-$index'),
                  offset: position,
                  child: Transform.rotate(angle: bee.rotation, child: child),
                );
              },
            ),
        ],
      ),
    );
  }
}

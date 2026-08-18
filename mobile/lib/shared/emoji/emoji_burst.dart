/// Emoji particle bursts, ported from desktop's `EmojiBurstProvider`
/// (`desktop/src/shared/ui/EmojiBurstProvider.tsx`).
///
/// Desktop paints to a fixed full-window `<canvas>` above everything and steps
/// the particles on `requestAnimationFrame`. Mobile does the same shape:
/// [EmojiBurstOverlay] installs one full-screen [CustomPaint] above the
/// navigator (so bursts survive route pushes and modal sheets), and a [Ticker]
/// steps the same physics.
///
/// The constants below are desktop's, unchanged — the motion is the product
/// decision, and drifting it would make the two clients feel different.
library;

import 'dart:math';

import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'positive_emoji.dart';

/// Particles per burst — desktop's `PICKER_PARTICLES_PER_BURST`.
const _particlesPerBurst = 5;

/// Frames a particle lives — desktop's `PICKER_PARTICLE_LIFE_FRAMES`.
const _particleLifeFrames = 108;

/// Desktop's `MAX_ACTIVE`, scaled down: a phone screen holds far fewer
/// particles before it reads as noise, and every one costs a `saveLayer`.
const _maxActiveParticles = 160;

/// Physics runs at a fixed 60Hz step regardless of display refresh rate.
/// Desktop's constants are per-frame deltas on a 60Hz assumption; stepping them
/// once per real frame would run the burst at double speed on a 120Hz panel.
const _stepSeconds = 1 / 60;

/// Cap on catch-up steps per frame so a stalled frame can't spiral.
const _maxStepsPerFrame = 4;

/// Glyph size the cached [TextPainter] is laid out at. Particles scale off this
/// rather than re-laying out text every frame.
const _glyphCachePx = 64.0;

/// The last emoji added as a reaction that is still waiting for its pill to
/// appear on screen.
///
/// Mobile reactions are not optimistic — `addReaction` awaits the relay and the
/// pill only exists once the event echoes back — so a burst fired at pick time
/// would play against a row that has not changed yet. Desktop solves this the
/// same way (`burstEmojiOnRender` in `MessageReactions.tsx`): remember the
/// pending emoji, and let the pill fire the burst from its own position the
/// frame it first renders as reacted-by-me.
@immutable
class PendingReactionBurst {
  final String messageId;
  final String emoji;

  const PendingReactionBurst({required this.messageId, required this.emoji});

  @override
  bool operator ==(Object other) =>
      other is PendingReactionBurst &&
      other.messageId == messageId &&
      other.emoji == emoji;

  @override
  int get hashCode => Object.hash(messageId, emoji);
}

/// One slot, not a queue: a burst is a flourish on the reaction you just added,
/// and two adds in flight at once means the newer one is the interesting one.
class PendingReactionBurstNotifier extends Notifier<PendingReactionBurst?> {
  @override
  PendingReactionBurst? build() => null;

  /// Arm a burst for [emoji] on [messageId]. No-op for emoji that don't
  /// celebrate, so callers don't have to gate themselves.
  void arm(String messageId, String emoji) {
    if (!isPositiveEmojiParticle(emoji)) return;
    state = PendingReactionBurst(messageId: messageId, emoji: emoji);
  }

  /// Claim the pending burst if it is [target], returning whether the caller
  /// won. The same message can be on screen twice (a thread pushed over its
  /// channel), so exactly one pill must fire.
  bool claim(PendingReactionBurst target) {
    if (state != target) return false;
    state = null;
    return true;
  }
}

final pendingReactionBurstProvider =
    NotifierProvider<PendingReactionBurstNotifier, PendingReactionBurst?>(
      PendingReactionBurstNotifier.new,
    );

class _Particle {
  double x;
  double y;
  double xv;
  double yv;
  double rotation;
  double spin;
  double scale;
  double opacity;
  double life;
  final double maxLife;
  final String emoji;
  final double fontSize;
  final double gravity;

  _Particle({
    required this.x,
    required this.y,
    required this.xv,
    required this.yv,
    required this.rotation,
    required this.spin,
    required this.scale,
    required this.opacity,
    required this.life,
    required this.maxLife,
    required this.emoji,
    required this.fontSize,
    required this.gravity,
  });

  /// Desktop's `updateParticle`. Returns false once the particle is spent.
  bool step() {
    life -= 1;
    rotation += spin;
    yv += gravity;
    xv *= 0.965;
    yv *= 0.998;
    x += xv;
    y += yv;
    scale += (1 - scale) * 0.28;

    final lifeRatio = life / maxLife;
    if (lifeRatio < 0.24) {
      opacity = max(0, lifeRatio / 0.24);
    }

    return life > 0 && opacity > 0.02;
  }
}

/// Holds the live particles and notifies the painter each step.
///
/// Deliberately not a `Notifier` over a particle list: the particles mutate 60
/// times a second and must never rebuild a widget. Listeners are the painter's
/// repaint signal only.
class EmojiBurstController extends ChangeNotifier {
  final List<_Particle> _particles = [];
  final Random _random;

  /// Set when a spawn arrives so the overlay knows to start its ticker.
  VoidCallback? onSpawn;

  EmojiBurstController({Random? random}) : _random = random ?? Random();

  bool get hasParticles => _particles.isNotEmpty;

  /// Spawn a burst of [emoji] centred on [origin] (global coordinates).
  /// Mirrors desktop's `spawnPickerEmojiBurst`.
  void burst(String emoji, Offset origin) {
    final trimmed = emoji.trim();
    if (trimmed.isEmpty) return;
    if (_particles.length + _particlesPerBurst > _maxActiveParticles) return;

    for (var i = 0; i < _particlesPerBurst; i += 1) {
      final horizontalDrift = (_random.nextDouble() - 0.5) * 4.4;
      final initialLift = 2.1 + _random.nextDouble() * 2.35;

      _particles.add(
        _Particle(
          x: origin.dx,
          y: origin.dy,
          xv: horizontalDrift,
          yv: -initialLift,
          rotation: (_random.nextDouble() - 0.5) * 22,
          spin: (_random.nextDouble() - 0.5) * 5.2,
          scale: 0.25,
          opacity: 1,
          life: _particleLifeFrames.toDouble(),
          maxLife: _particleLifeFrames.toDouble(),
          emoji: trimmed,
          fontSize: 18 + (_random.nextDouble() * 24).ceilToDouble(),
          // Negative: the burst floats up and keeps accelerating upward.
          gravity: -(0.018 + _random.nextDouble() * 0.018),
        ),
      );
    }

    onSpawn?.call();
    notifyListeners();
  }

  /// Advance one fixed step. Returns whether any particles remain.
  bool step() {
    for (var i = _particles.length - 1; i >= 0; i -= 1) {
      if (!_particles[i].step()) {
        _particles[i] = _particles.last;
        _particles.removeLast();
      }
    }
    notifyListeners();
    return _particles.isNotEmpty;
  }

  void clear() {
    if (_particles.isEmpty) return;
    _particles.clear();
    notifyListeners();
  }

  @override
  void dispose() {
    _particles.clear();
    super.dispose();
  }
}

final emojiBurstControllerProvider = Provider<EmojiBurstController>((ref) {
  final controller = EmojiBurstController();
  ref.onDispose(controller.dispose);
  return controller;
});

/// Fire a burst of [emoji] centred on [context]'s own box.
///
/// The convenience the call sites actually want: they have a build context for
/// the pill or tile that was tapped and no reason to know about global
/// coordinates. Silently does nothing for emoji outside the positive set or
/// when the platform asks for reduced motion, so callers stay simple.
void burstEmojiFromContext(
  WidgetRef ref,
  BuildContext context,
  String emoji, {
  bool requirePositive = true,
}) {
  if (requirePositive && !isPositiveEmojiParticle(emoji)) return;
  if (MediaQuery.maybeDisableAnimationsOf(context) ?? false) return;

  final box = context.findRenderObject();
  if (box is! RenderBox || !box.hasSize) return;
  final origin = box.localToGlobal(box.size.center(Offset.zero));
  ref.read(emojiBurstControllerProvider).burst(emoji, origin);
}

/// Full-screen particle layer. Install once, above the navigator, so bursts
/// keep playing over pushed routes and modal sheets.
class EmojiBurstOverlay extends HookConsumerWidget {
  final Widget child;

  const EmojiBurstOverlay({super.key, required this.child});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = ref.watch(emojiBurstControllerProvider);
    final tickerProvider = useSingleTickerProvider();

    // Ticker + leftover time live in refs: stepping physics must not rebuild
    // this widget, which wraps the entire app.
    final leftover = useRef(0.0);
    final lastElapsed = useRef(Duration.zero);

    final ticker = useMemoized(() {
      late final Ticker created;
      created = tickerProvider.createTicker((elapsed) {
        final delta = elapsed - lastElapsed.value;
        lastElapsed.value = elapsed;
        leftover.value += delta.inMicroseconds / Duration.microsecondsPerSecond;

        var steps = 0;
        var alive = controller.hasParticles;
        while (leftover.value >= _stepSeconds && steps < _maxStepsPerFrame) {
          leftover.value -= _stepSeconds;
          steps += 1;
          alive = controller.step();
          if (!alive) break;
        }
        // Don't bank unspent time from a stall — it would fast-forward the
        // next burst.
        if (steps >= _maxStepsPerFrame) leftover.value = 0;

        if (!alive) {
          created.stop();
          lastElapsed.value = Duration.zero;
          leftover.value = 0;
        }
      });
      return created;
    }, [tickerProvider, controller]);

    useEffect(() => ticker.dispose, [ticker]);

    useEffect(() {
      void start() {
        if (ticker.isActive) return;
        lastElapsed.value = Duration.zero;
        leftover.value = 0;
        ticker.start();
      }

      controller.onSpawn = start;
      if (controller.hasParticles) start();
      return () {
        if (controller.onSpawn == start) controller.onSpawn = null;
      };
    }, [ticker, controller]);

    return Stack(
      children: [
        child,
        Positioned.fill(
          child: IgnorePointer(
            child: ExcludeSemantics(
              child: RepaintBoundary(
                child: CustomPaint(painter: _EmojiBurstPainter(controller)),
              ),
            ),
          ),
        ),
      ],
    );
  }
}

/// Laid-out glyphs, keyed by emoji. Emoji are drawn thousands of times per
/// burst; laying out the text once and scaling the canvas is the whole trick.
final Map<String, TextPainter> _glyphCache = {};

/// Bounded so a chatty channel full of distinct reactions can't grow it
/// without limit. Bursts are visual sugar — evicting the whole cache costs one
/// re-layout per emoji still on screen.
const _glyphCacheLimit = 64;

TextPainter _glyphFor(String emoji) {
  final cached = _glyphCache[emoji];
  if (cached != null) return cached;
  if (_glyphCache.length >= _glyphCacheLimit) _glyphCache.clear();

  final painter = TextPainter(
    text: TextSpan(
      text: emoji,
      style: const TextStyle(fontSize: _glyphCachePx),
    ),
    textDirection: TextDirection.ltr,
  )..layout();
  _glyphCache[emoji] = painter;
  return painter;
}

/// Drop the cached glyph layouts. Exposed for tests; the cache is not
/// community- or account-scoped, so nothing else needs to reset it.
@visibleForTesting
void clearEmojiBurstGlyphCache() => _glyphCache.clear();

class _EmojiBurstPainter extends CustomPainter {
  final EmojiBurstController controller;

  _EmojiBurstPainter(this.controller) : super(repaint: controller);

  @override
  void paint(Canvas canvas, Size size) {
    for (final particle in controller._particles) {
      final glyph = _glyphFor(particle.emoji);
      final drawSize = particle.fontSize * particle.scale;
      final scale = drawSize / _glyphCachePx;

      canvas.save();
      canvas.translate(particle.x, particle.y);
      canvas.rotate(particle.rotation * pi / 180);
      canvas.scale(scale);
      // Color fonts ignore TextStyle.color, so fade through a layer instead.
      if (particle.opacity < 1) {
        canvas.saveLayer(
          Rect.fromCenter(
            center: Offset.zero,
            width: glyph.width,
            height: glyph.height,
          ),
          Paint()..color = Colors.black.withValues(alpha: particle.opacity),
        );
      }
      glyph.paint(canvas, Offset(-glyph.width / 2, -glyph.height / 2));
      if (particle.opacity < 1) canvas.restore();
      canvas.restore();
    }
  }

  @override
  bool shouldRepaint(_EmojiBurstPainter oldDelegate) =>
      oldDelegate.controller != controller;
}

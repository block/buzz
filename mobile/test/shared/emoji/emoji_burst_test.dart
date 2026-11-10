import 'package:buzz/shared/emoji/emoji_burst.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _fire = '\u{1F525}';

/// Run the controller forward until it says it is done, capped so a physics
/// regression fails the test instead of hanging it.
int _runToCompletion(EmojiBurstController controller, {int maxSteps = 600}) {
  var steps = 0;
  while (controller.hasParticles && steps < maxSteps) {
    controller.step();
    steps += 1;
  }
  return steps;
}

void main() {
  group('EmojiBurstController', () {
    test('a burst spawns particles and eventually clears itself', () {
      final controller = EmojiBurstController();
      addTearDown(controller.dispose);

      expect(controller.hasParticles, isFalse);
      controller.burst(_fire, const Offset(100, 200));
      expect(controller.hasParticles, isTrue);

      final steps = _runToCompletion(controller);
      expect(controller.hasParticles, isFalse);
      // Desktop's particles live 108 frames and fade over the last 24%.
      expect(steps, lessThanOrEqualTo(108));
      expect(steps, greaterThan(60));
    });

    test('ignores a blank emoji', () {
      final controller = EmojiBurstController();
      addTearDown(controller.dispose);

      controller.burst('   ', Offset.zero);
      expect(controller.hasParticles, isFalse);
    });

    test('notifies on spawn and on every step', () {
      final controller = EmojiBurstController();
      addTearDown(controller.dispose);

      var notifications = 0;
      controller.addListener(() => notifications += 1);

      controller.burst(_fire, Offset.zero);
      expect(notifications, 1);

      controller.step();
      expect(notifications, 2);
    });

    test('calls onSpawn so the overlay can start its ticker', () {
      final controller = EmojiBurstController();
      addTearDown(controller.dispose);

      var spawns = 0;
      controller.onSpawn = () => spawns += 1;

      controller.burst(_fire, Offset.zero);
      controller.burst(_fire, Offset.zero);
      expect(spawns, 2);
    });

    test('caps the active particle count', () {
      final controller = EmojiBurstController();
      addTearDown(controller.dispose);

      // Far more bursts than the cap allows, with no steps in between so
      // nothing expires.
      for (var i = 0; i < 200; i += 1) {
        controller.burst(_fire, Offset.zero);
      }

      // Still bounded, and still animating rather than wedged.
      expect(controller.hasParticles, isTrue);
      _runToCompletion(controller);
      expect(controller.hasParticles, isFalse);
    });

    test('clear drops everything immediately', () {
      final controller = EmojiBurstController();
      addTearDown(controller.dispose);

      controller.burst(_fire, Offset.zero);
      controller.clear();
      expect(controller.hasParticles, isFalse);
    });
  });

  group('PendingReactionBurstNotifier', () {
    ProviderContainer container() {
      final result = ProviderContainer();
      addTearDown(result.dispose);
      return result;
    }

    test('arms only positive emoji', () {
      final ref = container();
      final notifier = ref.read(pendingReactionBurstProvider.notifier);

      notifier.arm('msg-1', '\u{1F44E}'); // 👎
      expect(ref.read(pendingReactionBurstProvider), isNull);

      notifier.arm('msg-1', _fire);
      expect(
        ref.read(pendingReactionBurstProvider),
        const PendingReactionBurst(messageId: 'msg-1', emoji: _fire),
      );
    });

    test('claim succeeds once, so one pill bursts when a message is on screen '
        'twice', () {
      final ref = container();
      final notifier = ref.read(pendingReactionBurstProvider.notifier);
      const target = PendingReactionBurst(messageId: 'msg-1', emoji: _fire);

      notifier.arm('msg-1', _fire);
      expect(notifier.claim(target), isTrue);
      expect(notifier.claim(target), isFalse);
      expect(ref.read(pendingReactionBurstProvider), isNull);
    });

    test('a claim for a different message or emoji does not consume it', () {
      final ref = container();
      final notifier = ref.read(pendingReactionBurstProvider.notifier);

      notifier.arm('msg-1', _fire);
      expect(
        notifier.claim(
          const PendingReactionBurst(messageId: 'msg-2', emoji: _fire),
        ),
        isFalse,
      );
      expect(
        notifier.claim(
          const PendingReactionBurst(messageId: 'msg-1', emoji: '\u{1F389}'),
        ),
        isFalse,
      );
      expect(ref.read(pendingReactionBurstProvider), isNotNull);
    });

    test('a newer arm replaces the pending one', () {
      final ref = container();
      final notifier = ref.read(pendingReactionBurstProvider.notifier);

      notifier.arm('msg-1', _fire);
      notifier.arm('msg-2', '\u{1F389}');
      expect(
        ref.read(pendingReactionBurstProvider),
        const PendingReactionBurst(messageId: 'msg-2', emoji: '\u{1F389}'),
      );
    });
  });

  group('EmojiBurstOverlay', () {
    testWidgets('paints its child and drains the burst', (tester) async {
      late EmojiBurstController controller;

      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            home: Consumer(
              builder: (context, ref, _) {
                controller = ref.watch(emojiBurstControllerProvider);
                return const EmojiBurstOverlay(child: Text('body'));
              },
            ),
          ),
        ),
      );

      expect(find.text('body'), findsOneWidget);

      controller.burst(_fire, const Offset(120, 240));
      expect(controller.hasParticles, isTrue);

      // Physics advances a bounded number of fixed steps per frame (no
      // catch-up spiral), so drain it a frame at a time — 130 frames is past
      // the 108-frame particle life.
      await tester.pump();
      for (var frame = 0; frame < 130; frame += 1) {
        await tester.pump(const Duration(milliseconds: 16));
      }
      expect(controller.hasParticles, isFalse);
    });

    testWidgets('the overlay never takes a tap from the app below', (
      tester,
    ) async {
      var taps = 0;
      late EmojiBurstController controller;

      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            home: Consumer(
              builder: (context, ref, _) {
                controller = ref.watch(emojiBurstControllerProvider);
                return EmojiBurstOverlay(
                  child: Center(
                    child: GestureDetector(
                      onTap: () => taps += 1,
                      child: const Text('tap me'),
                    ),
                  ),
                );
              },
            ),
          ),
        ),
      );

      controller.burst(_fire, const Offset(200, 400));
      await tester.pump();

      await tester.tap(find.text('tap me'));
      expect(taps, 1);
    });
  });
}

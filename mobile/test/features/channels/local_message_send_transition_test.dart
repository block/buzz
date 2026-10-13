import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/local_message_send_animation_provider.dart';
import 'package:buzz/features/channels/local_message_send_transition.dart';
import 'package:buzz/features/channels/local_send_row.dart';

Widget _testable({bool animate = true, bool reduceMotion = false}) =>
    ProviderScope(
      child: MaterialApp(
        home: MediaQuery(
          data: MediaQueryData(disableAnimations: reduceMotion),
          child: Scaffold(
            body: Align(
              alignment: Alignment.bottomLeft,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  const SizedBox(
                    key: ValueKey('history'),
                    width: 100,
                    height: 40,
                  ),
                  LocalMessageSendTransition(
                    animate: animate,
                    child: const SizedBox(
                      width: 100,
                      height: 80,
                      child: Align(
                        alignment: Alignment.bottomLeft,
                        child: SizedBox(
                          key: ValueKey('avatar'),
                          width: 40,
                          height: 40,
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
    );

void main() {
  test('local send claims expire before later navigation', () {
    final now = DateTime(2026);
    expect(
      isRecentLocalMessageSendAnimation(
        {'id': now},
        'id',
        now: now.add(const Duration(milliseconds: 999)),
      ),
      isTrue,
    );
    expect(
      isRecentLocalMessageSendAnimation(
        {'id': now},
        'id',
        now: now.add(const Duration(seconds: 2)),
      ),
      isFalse,
    );
  });
  testWidgets('history moves up while the avatar remains anchored', (
    tester,
  ) async {
    await tester.pumpWidget(_testable());
    final avatar = tester.getRect(find.byKey(const ValueKey('avatar')));
    final history = tester.getRect(find.byKey(const ValueKey('history')));
    expect(tester.getSize(find.byType(LocalSendRow)).height, 0);
    await tester.pump(localMessageSendTransitionDuration ~/ 2);
    expect(
      tester.getSize(find.byType(LocalSendRow)).height,
      inExclusiveRange(15, 35),
    );
    expect(tester.getRect(find.byKey(const ValueKey('avatar'))), avatar);
    expect(
      tester.getTopLeft(find.byKey(const ValueKey('history'))).dy,
      closeTo(
        history.top - tester.getSize(find.byType(LocalSendRow)).height,
        0.1,
      ),
    );
    await tester.pumpAndSettle();
    expect(tester.getSize(find.byType(LocalSendRow)).height, 80);
    expect(tester.getRect(find.byKey(const ValueKey('avatar'))), avatar);
    expect(
      tester.getTopLeft(find.byKey(const ValueKey('history'))).dy,
      history.top - 80,
    );
  });
  testWidgets('contraction leads the overlapping lift and settles first', (
    tester,
  ) async {
    await tester.pumpWidget(_testable());
    LocalSendProgress motion() => tester.widget(find.byType(LocalSendProgress));
    await tester.pump(localMessageSendTransitionDuration ~/ 4);
    expect(motion().widthProgress, greaterThan(motion().progress));
    expect(motion().progress, greaterThan(0));
    await tester.pump(localMessageSendTransitionDuration * 0.375);
    expect(motion().widthProgress, 1);
    expect(motion().progress, inExclusiveRange(0, 1));
    await tester.pump(localMessageSendTransitionDuration * 0.375);
    expect(motion().progress, 1);
  });
  for (final reduced in [false, true]) {
    testWidgets('settled or reduced motion does not expand, reduced=$reduced', (
      tester,
    ) async {
      await tester.pumpWidget(
        _testable(animate: reduced, reduceMotion: reduced),
      );
      expect(tester.getSize(find.byType(LocalSendRow)).height, 80);
      await tester.pump(const Duration(milliseconds: 100));
      expect(tester.getSize(find.byType(LocalSendRow)).height, 80);
    });
  }
}

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/local_message_send_animation_provider.dart';
import 'package:buzz/features/channels/local_message_send_transition.dart';

Widget _testable({
  bool animate = true,
  bool reduceMotion = false,
  double startOffsetFactor = localMessageSendTransitionStartOffset,
}) => MaterialApp(
  home: MediaQuery(
    data: MediaQueryData(disableAnimations: reduceMotion),
    child: Scaffold(
      body: LocalMessageSendTransition(
        animate: animate,
        startOffsetFactor: startOffsetFactor,
        child: const SizedBox(key: ValueKey('message'), width: 100, height: 40),
      ),
    ),
  ),
);

void main() {
  Finder sendTranslation() => find.descendant(
    of: find.byType(LocalMessageSendTransition),
    matching: find.byType(FractionalTranslation),
  );

  test('local send claims expire before a later navigation', () {
    final markedAt = DateTime(2026);
    final animations = {'event': markedAt};

    expect(
      isRecentLocalMessageSendAnimation(
        animations,
        'event',
        now: markedAt.add(const Duration(milliseconds: 999)),
      ),
      isTrue,
    );
    expect(
      isRecentLocalMessageSendAnimation(
        animations,
        'event',
        now: markedAt.add(const Duration(seconds: 2)),
      ),
      isFalse,
    );
  });

  testWidgets('local send rises from behind the composer', (tester) async {
    await tester.pumpWidget(_testable());

    var transition = tester.widget<FractionalTranslation>(sendTranslation());
    expect(
      transition.translation,
      const Offset(0, localMessageSendTransitionStartOffset),
    );
    expect(
      find.descendant(
        of: find.byType(LocalMessageSendTransition),
        matching: find.byType(FadeTransition),
      ),
      findsNothing,
    );

    await tester.pump(localMessageSendTransitionDuration ~/ 2);
    transition = tester.widget<FractionalTranslation>(sendTranslation());
    expect(transition.translation.dy, greaterThan(0));
    expect(
      transition.translation.dy,
      lessThan(localMessageSendTransitionStartOffset),
    );

    await tester.pump(localMessageSendTransitionDuration);
    transition = tester.widget<FractionalTranslation>(sendTranslation());
    expect(transition.translation, Offset.zero);
  });

  testWidgets('an avatar row starts deeper behind the composer', (
    tester,
  ) async {
    await tester.pumpWidget(
      _testable(startOffsetFactor: localMessageSendTransitionAvatarStartOffset),
    );

    final transition = tester.widget<FractionalTranslation>(sendTranslation());
    expect(
      transition.translation,
      const Offset(0, localMessageSendTransitionAvatarStartOffset),
    );
  });

  testWidgets('ordinary messages do not animate', (tester) async {
    await tester.pumpWidget(_testable(animate: false));

    final transition = tester.widget<FractionalTranslation>(sendTranslation());
    expect(transition.translation, Offset.zero);
  });

  testWidgets('reduced motion skips the send translation', (tester) async {
    await tester.pumpWidget(_testable(reduceMotion: true));

    final transition = tester.widget<FractionalTranslation>(sendTranslation());
    expect(transition.translation, Offset.zero);
  });
}

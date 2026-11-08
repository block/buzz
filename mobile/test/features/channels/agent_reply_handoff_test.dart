import 'package:buzz/features/channels/agent_reply_handoff.dart';
import 'package:buzz/features/channels/agent_typing_bubble.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final reduced in [false, true]) {
    testWidgets('reply grows from bubble with reduced motion $reduced', (
      tester,
    ) async {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: MediaQuery(
            data: MediaQueryData(disableAnimations: reduced),
            child: const Scaffold(
              body: Align(
                alignment: Alignment.topLeft,
                child: AgentReplyHandoff(
                  animate: true,
                  child: SizedBox(
                    key: ValueKey('reply'),
                    width: 220,
                    height: 120,
                    child: Text('Agent reply'),
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      expect(
        find.byType(AgentTypingBubble),
        reduced ? findsNothing : findsOneWidget,
      );
      if (!reduced) {
        final bubble = tester.getSize(find.byType(AnimatedSize));
        expect(bubble.width, lessThan(220));
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 70));
        final during = tester.getSize(find.byType(AnimatedSize));
        expect(during.width, greaterThan(bubble.width));
        expect(during.width, lessThan(220));
      }
      await tester.pumpAndSettle();
      expect(find.byType(AgentTypingBubble), findsNothing);
      expect(find.text('Agent reply'), findsOneWidget);
      expect(
        tester.getSize(find.byKey(const ValueKey('reply'))),
        const Size(220, 120),
      );
    });
  }
}

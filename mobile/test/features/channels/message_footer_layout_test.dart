import 'package:buzz/features/channels/message_footer_layout.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets(
    'overlapping reaction area is tappable and reserves its protrusion',
    (tester) async {
      var taps = 0;
      await tester.pumpWidget(
        MaterialApp(
          home: Align(
            alignment: Alignment.topLeft,
            child: MessageFooterLayout(
              overlap: 8,
              footer: GestureDetector(
                onTap: () => taps++,
                behavior: HitTestBehavior.opaque,
                child: const SizedBox(
                  key: ValueKey('reactions'),
                  width: 100,
                  height: 32,
                ),
              ),
              child: const SizedBox(
                key: ValueKey('body'),
                width: 200,
                height: 60,
              ),
            ),
          ),
        ),
      );
      final body = tester.getRect(find.byKey(const ValueKey('body')));
      final badge = tester.getRect(find.byKey(const ValueKey('reactions')));
      expect(badge.top, body.bottom - 8);
      expect(tester.getSize(find.byType(MessageFooterLayout)).height, 84);
      await tester.tapAt(Offset(badge.left + 10, badge.top + 2));
      expect(taps, 1);
    },
  );
}

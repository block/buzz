import 'package:buzz/features/channels/composer_send_origin.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('captures the submitted composer before its layout collapses', (
    tester,
  ) async {
    final origin = ComposerSendOrigin();
    final key = GlobalKey();
    expect(origin.recentRect, isNull);
    Future<void> layout(double height) => tester.pumpWidget(
      MaterialApp(
        home: Align(
          alignment: Alignment.bottomCenter,
          child: SizedBox(key: key, width: 320, height: height),
        ),
      ),
    );
    await layout(120);
    final submitted = tester.getRect(find.byKey(key));
    origin.capture(key);
    expect(origin.recentRect, submitted);
    await layout(44);
    expect(origin.recentRect, submitted);
    expect(origin.recentRect, isNot(tester.getRect(find.byKey(key))));
  });
}

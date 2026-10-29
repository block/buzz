import 'package:flutter_test/flutter_test.dart';

Future<void> twoFingerDoubleTap(WidgetTester tester, Offset position) async {
  for (final time in [100, 200]) {
    final first = await tester.createGesture();
    final second = await tester.createGesture();
    await first.down(position, timeStamp: Duration(milliseconds: time));
    await second.down(
      position + const Offset(40, 0),
      timeStamp: Duration(milliseconds: time + 5),
    );
    await first.up(timeStamp: Duration(milliseconds: time + 20));
    await second.up(timeStamp: Duration(milliseconds: time + 25));
  }
  await tester.pumpAndSettle();
}

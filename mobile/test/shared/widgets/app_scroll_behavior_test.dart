import 'dart:ui' show PointerDeviceKind;

import 'package:buzz/shared/widgets/app_scroll_behavior.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

Widget _list({ScrollBehavior? behavior}) {
  return MaterialApp(
    scrollBehavior: behavior,
    home: Scaffold(
      body: ListView(
        children: [
          for (var i = 0; i < 40; i++)
            SizedBox(height: 60, child: Text('row $i')),
        ],
      ),
    ),
  );
}

Future<double> _mouseDrag(WidgetTester tester) async {
  final gesture = await tester.startGesture(
    tester.getCenter(find.byType(ListView)),
    kind: PointerDeviceKind.mouse,
  );
  await gesture.moveBy(const Offset(0, -200));
  await tester.pump();
  await gesture.up();
  await tester.pumpAndSettle();
  return tester.state<ScrollableState>(find.byType(Scrollable)).position.pixels;
}

void main() {
  testWidgets('keeps the touch-like drag devices', (tester) async {
    const behavior = AppScrollBehavior();
    expect(
      behavior.dragDevices,
      containsAll(const MaterialScrollBehavior().dragDevices),
    );
    expect(behavior.dragDevices, contains(PointerDeviceKind.mouse));
  });

  testWidgets('a mouse drag scrolls a list under the app behavior', (
    tester,
  ) async {
    await tester.pumpWidget(_list(behavior: const AppScrollBehavior()));
    expect(await _mouseDrag(tester), greaterThan(0));
  });

  testWidgets('the Flutter default still ignores a mouse drag', (tester) async {
    // Guards the premise: if Flutter ever starts accepting mouse drags on
    // its own, the override becomes dead weight and this test says so.
    await tester.pumpWidget(_list());
    expect(await _mouseDrag(tester), 0);
  });

  testWidgets('a finger drag still scrolls under the app behavior', (
    tester,
  ) async {
    await tester.pumpWidget(_list(behavior: const AppScrollBehavior()));
    await tester.drag(find.byType(ListView), const Offset(0, -200));
    await tester.pumpAndSettle();
    expect(
      tester.state<ScrollableState>(find.byType(Scrollable)).position.pixels,
      greaterThan(0),
    );
  });
}

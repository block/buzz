import 'package:buzz/shared/widgets/keyboard_dismiss_on_drag.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// A focused field inside a `Scaffold` body, which is the only arrangement
/// either message list ever runs in.
Widget _testable({required FocusNode focusNode}) {
  return MaterialApp(
    home: Scaffold(
      body: Column(
        children: [
          TextField(focusNode: focusNode),
          Expanded(
            child: KeyboardDismissOnDrag(
              child: ListView(
                children: [
                  for (var i = 0; i < 40; i++)
                    SizedBox(height: 60, child: Text('row $i')),
                ],
              ),
            ),
          ),
        ],
      ),
    ),
  );
}

/// Raise the keyboard the way the platform does — on the view, not via a
/// synthetic `MediaQuery` wrapper. It has to travel the real path, because
/// `Scaffold` strips the bottom view inset from its body once it has resized to
/// account for it, and that stripping is the whole reason this widget broke.
void _raiseKeyboard(WidgetTester tester) {
  tester.view.viewInsets = const FakeViewPadding(bottom: 300);
  addTearDown(tester.view.reset);
}

Future<void> _dragDown(
  WidgetTester tester,
  double distance, {
  String on = 'row 3',
}) async {
  final gesture = await tester.startGesture(tester.getCenter(find.text(on)));
  // Several small moves, the way a real finger reports — the accumulator has to
  // add them up rather than needing one big delta.
  final step = distance / 6;
  for (var i = 0; i < 6; i++) {
    await gesture.moveBy(Offset(0, step));
    await tester.pump();
  }
  await gesture.up();
  await tester.pump();
}

void main() {
  group('KeyboardDismissOnDrag', () {
    testWidgets('a deliberate downward drag drops focus', (tester) async {
      final focusNode = FocusNode();
      addTearDown(focusNode.dispose);
      _raiseKeyboard(tester);
      await tester.pumpWidget(_testable(focusNode: focusNode));
      focusNode.requestFocus();
      await tester.pump();
      expect(focusNode.hasFocus, isTrue);

      await _dragDown(tester, keyboardDismissDragThreshold * 2);

      // At rest the list can't scroll down, so this gesture is pure overscroll
      // — the case that reports `OverscrollNotification` rather than
      // `ScrollUpdateNotification`, and the one a focused composer is usually
      // sitting in.
      expect(focusNode.hasFocus, isFalse);
    });

    testWidgets('an ordinary downward scroll dismisses too', (tester) async {
      final focusNode = FocusNode();
      addTearDown(focusNode.dispose);
      _raiseKeyboard(tester);
      await tester.pumpWidget(_testable(focusNode: focusNode));
      focusNode.requestFocus();
      await tester.pump();

      // Move off the resting edge so dragging down actually moves the position,
      // covering the `ScrollUpdateNotification` path rather than overscroll.
      await tester.drag(find.text('row 3'), const Offset(0, -400));
      await tester.pumpAndSettle();
      expect(focusNode.hasFocus, isTrue);

      await _dragDown(tester, keyboardDismissDragThreshold * 2, on: 'row 10');

      expect(focusNode.hasFocus, isFalse);
    });

    testWidgets('a short scroll leaves the keyboard alone', (tester) async {
      final focusNode = FocusNode();
      addTearDown(focusNode.dispose);
      _raiseKeyboard(tester);
      await tester.pumpWidget(_testable(focusNode: focusNode));
      focusNode.requestFocus();
      await tester.pump();

      await _dragDown(tester, keyboardDismissDragThreshold / 2);

      expect(focusNode.hasFocus, isTrue);
    });

    testWidgets('an upward drag never dismisses, however far it goes', (
      tester,
    ) async {
      final focusNode = FocusNode();
      addTearDown(focusNode.dispose);
      _raiseKeyboard(tester);
      await tester.pumpWidget(_testable(focusNode: focusNode));
      focusNode.requestFocus();
      await tester.pump();

      await _dragDown(tester, -keyboardDismissDragThreshold * 4);

      expect(focusNode.hasFocus, isTrue);
    });

    testWidgets('with the keyboard already down, focus is left as it is', (
      tester,
    ) async {
      final focusNode = FocusNode();
      addTearDown(focusNode.dispose);
      // No _raiseKeyboard: the keyboard is down.
      await tester.pumpWidget(_testable(focusNode: focusNode));
      focusNode.requestFocus();
      await tester.pump();

      await _dragDown(tester, keyboardDismissDragThreshold * 2);

      // Nothing to dismiss, so don't yank focus out of whatever holds it.
      expect(focusNode.hasFocus, isTrue);
    });
  });
}

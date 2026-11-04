import 'package:buzz/shared/widgets/forward_history.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  setUp(forwardHistory.clear);

  testWidgets('right-edge swipe retraces popped pages until a new push', (
    tester,
  ) async {
    final navigator = GlobalKey<NavigatorState>();
    Route<void> page(String name) => MaterialPageRoute<void>(
      builder: (_) => Scaffold(body: Center(child: Text(name))),
    );
    await tester.pumpWidget(
      MaterialApp(
        navigatorKey: navigator,
        navigatorObservers: [forwardHistory],
        builder: (_, child) => ForwardSwipeHost(child: child!),
        home: const Scaffold(body: Center(child: Text('home'))),
      ),
    );
    final size = tester.view.physicalSize / tester.view.devicePixelRatio;
    Future<void> swipeForward() async {
      await tester.dragFrom(
        Offset(size.width - 4, size.height / 2),
        const Offset(-120, 0),
      );
      await tester.pumpAndSettle();
    }

    // Nothing to go forward to: the edge is left to the app.
    await swipeForward();
    expect(find.text('home'), findsOneWidget);

    navigator.currentState!.push(page('channel'));
    await tester.pumpAndSettle();
    navigator.currentState!.push(page('thread'));
    await tester.pumpAndSettle();
    navigator.currentState!.pop();
    await tester.pumpAndSettle();
    navigator.currentState!.pop();
    await tester.pumpAndSettle();
    expect(find.text('home'), findsOneWidget);

    // A short drag does nothing; forward restores channel, then thread.
    await tester.dragFrom(
      Offset(size.width - 4, size.height / 2),
      const Offset(-10, 0),
    );
    await tester.pumpAndSettle();
    expect(find.text('home'), findsOneWidget);
    await swipeForward();
    expect(find.text('channel'), findsOneWidget);
    await swipeForward();
    expect(find.text('thread'), findsOneWidget);
    await swipeForward();
    expect(find.text('thread'), findsOneWidget);

    // A dialog keeps history; opening a different page discards it.
    navigator.currentState!.pop();
    await tester.pumpAndSettle();
    showDialog<void>(
      context: navigator.currentContext!,
      builder: (_) => const AlertDialog(content: Text('dialog')),
    );
    await tester.pumpAndSettle();
    navigator.currentState!.pop();
    await tester.pumpAndSettle();
    expect(forwardHistory.canGoForward.value, isTrue);
    navigator.currentState!.push(page('settings'));
    await tester.pumpAndSettle();
    expect(forwardHistory.canGoForward.value, isFalse);
    await swipeForward();
    expect(find.text('settings'), findsOneWidget);
  });
}

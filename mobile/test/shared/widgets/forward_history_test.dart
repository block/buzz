import 'package:buzz/shared/widgets/forward_history.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter_test/flutter_test.dart';

/// A page that remembers a counter across a back-then-forward round trip.
class _CounterPage extends HookWidget {
  const _CounterPage(this.name);

  final String name;

  @override
  Widget build(BuildContext context) {
    final restoration = useForwardRestoration<int>();
    final count = useState(restoration.restored ?? 0);
    restoration.save = () => count.value;
    return Scaffold(
      body: Center(
        child: TextButton(
          onPressed: () => count.value++,
          child: Text('$name ${count.value}'),
        ),
      ),
    );
  }
}

void main() {
  setUp(forwardHistory.clear);

  testWidgets('right-edge swipe retraces popped pages as they were left', (
    tester,
  ) async {
    final navigator = GlobalKey<NavigatorState>();
    Route<void> page(String name) =>
        MaterialPageRoute<void>(builder: (_) => _CounterPage(name));
    await tester.pumpWidget(
      MaterialApp(
        navigatorKey: navigator,
        navigatorObservers: [forwardHistory],
        builder: (_, child) => ForwardSwipeHost(child: child!),
        home: const Scaffold(body: Center(child: Text('home'))),
      ),
    );
    final size = tester.view.physicalSize / tester.view.devicePixelRatio;
    final edge = Offset(size.width - 4, size.height / 2);
    // Slow drags, so the release decides by distance rather than speed.
    Future<void> swipeForward(double fraction) async {
      await tester.timedDragFrom(
        edge,
        Offset(-size.width * fraction, 0),
        const Duration(seconds: 1),
      );
      await tester.pumpAndSettle();
    }

    Future<void> go(void Function(NavigatorState) action) async {
      action(navigator.currentState!);
      await tester.pumpAndSettle();
    }

    // Nothing to go forward to: the edge is left to the app.
    await swipeForward(0.7);
    expect(find.text('home'), findsOneWidget);

    await go((n) => n.push(page('channel')));
    await tester.tap(find.text('channel 0'));
    await tester.pump();
    await go((n) => n.push(page('thread')));
    await go((n) => n.pop());
    await go((n) => n.pop());
    expect(find.text('home'), findsOneWidget);

    // The page follows the finger, and letting go early puts it back.
    final gesture = await tester.startGesture(edge);
    await gesture.moveBy(const Offset(-20, 0));
    await gesture.moveBy(Offset(-size.width * 0.3, 0));
    await tester.pump();
    await tester.pump();
    final incoming = find.text('channel 1');
    final left = tester
        .getTopLeft(
          find.ancestor(of: incoming, matching: find.byType(Scaffold)),
        )
        .dx;
    expect(incoming, findsOneWidget);
    expect(left, inExclusiveRange(size.width * 0.5, size.width * 0.8));
    await gesture.up();
    await tester.pumpAndSettle();
    expect(incoming, findsNothing);
    expect(forwardHistory.canGoForward.value, isTrue);

    // Past half way it lands, restoring the page's own state, then the
    // thread that was above it.
    await swipeForward(0.7);
    expect(find.text('channel 1'), findsOneWidget);
    await swipeForward(0.7);
    expect(find.text('thread 0'), findsOneWidget);
    expect(forwardHistory.canGoForward.value, isFalse);

    // A dialog keeps history; opening a different page discards it.
    await go((n) => n.pop());
    showDialog<void>(
      context: navigator.currentContext!,
      builder: (_) => const AlertDialog(content: Text('dialog')),
    );
    await tester.pumpAndSettle();
    await go((n) => n.pop());
    expect(forwardHistory.canGoForward.value, isTrue);
    await go((n) => n.push(page('settings')));
    expect(forwardHistory.canGoForward.value, isFalse);
    await swipeForward(0.7);
    expect(find.text('settings 0'), findsOneWidget);
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));
}

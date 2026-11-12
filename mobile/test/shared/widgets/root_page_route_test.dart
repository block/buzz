import 'package:buzz/shared/widgets/root_page_route.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('a stray pop never empties the navigator', (tester) async {
    final navigator = GlobalKey<NavigatorState>();
    await tester.pumpWidget(
      MaterialApp(
        navigatorKey: navigator,
        onGenerateRoute: (settings) => RootPageRoute<void>(
          settings: settings,
          builder: (_) => const Scaffold(body: Text('Home')),
        ),
      ),
    );

    // Routes above Home still pop normally.
    navigator.currentState!.push(
      MaterialPageRoute<void>(
        builder: (_) => const Scaffold(body: Text('Chat')),
      ),
    );
    await tester.pumpAndSettle();
    navigator.currentState!.pop();
    await tester.pumpAndSettle();
    expect(find.text('Chat'), findsNothing);
    expect(find.text('Home'), findsOneWidget);

    // A pop that reaches Home is reported and ignored.
    navigator.currentState!.pop();
    await tester.pumpAndSettle();
    expect(tester.takeException(), isA<StateError>());
    expect(find.text('Home'), findsOneWidget);
  });
}

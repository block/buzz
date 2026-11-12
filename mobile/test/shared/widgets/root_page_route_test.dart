import 'package:buzz/shared/widgets/root_page_route.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('a stray pop never empties the navigator', (tester) async {
    final navigator = GlobalKey<NavigatorState>();
    final scaffold = GlobalKey<ScaffoldState>();
    await tester.pumpWidget(
      MaterialApp(
        navigatorKey: navigator,
        onGenerateRoute: (settings) => RootPageRoute<void>(
          settings: settings,
          builder: (_) => Scaffold(
            key: scaffold,
            drawer: const Drawer(child: Text('Drawer')),
            body: const Text('Home'),
          ),
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

    // A drawer on Home still closes on back.
    scaffold.currentState!.openDrawer();
    await tester.pumpAndSettle();
    await navigator.currentState!.maybePop();
    await tester.pumpAndSettle();
    expect(find.text('Drawer'), findsNothing);
    expect(tester.takeException(), isNull);

    // A pop that reaches Home is reported and ignored.
    navigator.currentState!.pop();
    await tester.pumpAndSettle();
    expect(tester.takeException(), isA<StateError>());
    expect(find.text('Home'), findsOneWidget);
  });
}

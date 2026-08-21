import 'package:buzz/features/pairing/pairing_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:integration_test/integration_test.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('pairing code reveal, edit, and hide survives native rendering', (
    tester,
  ) async {
    await tester.pumpWidget(
      const ProviderScope(child: MaterialApp(home: PairingPage())),
    );
    await tester.pumpAndSettle();

    expect(find.text('Welcome to Buzz'), findsOneWidget);
    expect(find.byKey(const Key('pairing-code-input')), findsNothing);

    await tester.tap(find.byKey(const Key('pairing-code-toggle')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('pairing-code-input')), findsOneWidget);
    await tester.pump(const Duration(seconds: 1));

    await tester.enterText(
      find.byKey(const Key('pairing-code-input')),
      'nostrpair://native-review',
    );
    await tester.pump();
    expect(find.text('nostrpair://native-review'), findsOneWidget);
    expect(find.byKey(const Key('pairing-connect')), findsOneWidget);
    await tester.pump(const Duration(seconds: 1));

    await tester.tap(find.byKey(const Key('pairing-code-toggle')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('pairing-code-input')), findsNothing);
  });
}

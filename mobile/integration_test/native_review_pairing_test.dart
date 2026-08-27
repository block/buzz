import 'dart:io';

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
    debugPrint('BUZZ_NATIVE_REVIEW_RECORDING_READY');
    debugPrint('BUZZ_NATIVE_REVIEW_STATE:initial-hidden');
    final proceedUrl = Platform.environment['BUZZ_NATIVE_REVIEW_PROCEED_URL'];
    expect(proceedUrl, isNotNull);
    const proceedTimeout = Duration(minutes: 3);
    final client = HttpClient();
    try {
      final proceed = await client
          .getUrl(Uri.parse(proceedUrl!))
          .then((request) => request.close())
          .timeout(proceedTimeout);
      expect(proceed.statusCode, HttpStatus.noContent);
      await proceed.drain<void>().timeout(proceedTimeout);
    } finally {
      client.close(force: true);
    }

    await tester.tap(find.byKey(const Key('pairing-code-toggle')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('pairing-code-input')), findsOneWidget);
    debugPrint('BUZZ_NATIVE_REVIEW_STATE:revealed');
    await tester.pump(const Duration(seconds: 1));

    await tester.enterText(
      find.byKey(const Key('pairing-code-input')),
      'nostrpair://native-review',
    );
    await tester.pump();
    expect(find.text('nostrpair://native-review'), findsOneWidget);
    expect(find.byKey(const Key('pairing-connect')), findsOneWidget);
    debugPrint('BUZZ_NATIVE_REVIEW_STATE:edited');
    await tester.pump(const Duration(seconds: 1));

    await tester.tap(find.byKey(const Key('pairing-code-toggle')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('pairing-code-input')), findsNothing);
    debugPrint('BUZZ_NATIVE_REVIEW_STATE:final-hidden');
  });
}

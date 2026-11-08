import 'dart:async';

import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'agents_harness.dart';

void main() {
  testWidgets(
    'Agents does not ask to sign in while a saved session is restoring',
    (tester) async {
      final h = await AgentsHarness.create();
      h.registry.signedIn = false;
      final restore = Completer<void>();
      h.registry.restoreGate = restore;
      await h.open(tester, directory: []);

      const prompt = 'Reconnect to Builderlab';
      expect(h.controller.busy, isTrue);
      expect(find.text(prompt), findsNothing);
      expect(find.widgetWithText(FilledButton, 'Sign in'), findsNothing);
      expect(find.text('You haven’t added any agents yet.'), findsNothing);

      // No saved session after all: now the prompt is the way forward.
      restore.complete();
      await tester.pumpAndSettle();
      expect(h.controller.busy, isFalse);
      expect(find.text(prompt), findsOneWidget);
      // Remote agents may exist but are unlistable until sign-in, so an
      // empty listing must not claim there are none.
      expect(find.text('You haven’t added any agents yet.'), findsNothing);
    },
    skip: !HostedEnvironment.enabled,
  );
}

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
      await h.open(tester, directory: [existingDesktopKeys.public]);

      const prompt = 'Sign in to Builderlab to manage remote agents';
      expect(h.controller.busy, isTrue);
      expect(find.text(prompt), findsNothing);
      expect(find.widgetWithText(TextButton, 'Sign in'), findsNothing);
      expect(find.text('Manage on desktop'), findsNothing);

      // No saved session after all: now the prompt is the way forward.
      restore.complete();
      await tester.pumpAndSettle();
      expect(h.controller.busy, isFalse);
      expect(find.text(prompt), findsOneWidget);
    },
    skip: !HostedEnvironment.enabled,
  );
}

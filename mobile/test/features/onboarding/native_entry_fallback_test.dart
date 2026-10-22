import 'package:buzz/features/onboarding/self_hosted_entry_sheet.dart';
import 'package:buzz/shared/hosted/device_identity_source.dart';
import 'package:buzz/shared/hosted/self_hosted_entry.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

class _AvailableEntry extends SelfHostedEntry {
  @override
  Future<({List<DeviceIdentityChoice> choices, bool complete})>
  choices() async => (choices: const <DeviceIdentityChoice>[], complete: true);
}

void main() {
  testWidgets(
    'native presentation failure opens the usable Flutter join sheet',
    (tester) async {
      FlutterSecureStorage.setMockInitialValues({});
      const channel = MethodChannel('buzz/community_entry');
      final messenger = tester.binding.defaultBinaryMessenger;
      var presentations = 0;
      messenger.setMockMethodCallHandler(channel, (_) async {
        presentations++;
        throw PlatformException(code: 'presentation_failed');
      });
      addTearDown(() => messenger.setMockMethodCallHandler(channel, null));
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            selfHostedEntryProvider.overrideWithValue(_AvailableEntry()),
          ],
          child: Builder(
            builder: (context) => TextButton(
              onPressed: () => showSelfHostedEntry(
                context,
                relay: 'wss://example.com',
                name: 'Example',
              ),
              child: const Text('Open join'),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open join'));
      await tester.pumpAndSettle();
      expect(presentations, 1);
      expect(tester.takeException(), isNull);
      expect(find.byType(SelfHostedEntrySheet), findsOneWidget);
      expect(find.text('Import a Nostr key').hitTestable(), findsOneWidget);
      Navigator.of(tester.element(find.byType(SelfHostedEntrySheet))).pop();
      await tester.pumpAndSettle();
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );
}

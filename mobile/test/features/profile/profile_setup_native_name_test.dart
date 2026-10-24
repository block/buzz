import 'dart:async';

import 'package:buzz/features/profile/profile_setup_page.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/hosted_communities_fixture.dart';
import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('native profile saves 100 graphemes and rejects 101', (
    tester,
  ) async {
    FlutterSecureStorage.setMockInitialValues({});
    final fixture = HostedCommunitiesFixture();
    await fixture.initialize(widget: true);
    addTearDown(fixture.dispose);
    for (final grapheme in ['😀', 'e\u0301', '👨‍👩‍👧‍👦']) {
      for (final count in [100, 101]) {
        const channel = MethodChannel('buzz/profile_setup');
        final messenger = tester.binding.defaultBinaryMessenger;
        final presented = Completer<Map<dynamic, dynamic>>();
        final dismissed = Completer<String>();
        messenger.setMockMethodCallHandler(channel, (call) async {
          if (call.method == 'present') {
            presented.complete(call.arguments as Map);
            return dismissed.future;
          }
          return null;
        });
        addTearDown(() => messenger.setMockMethodCallHandler(channel, null));
        final writes = <String>[];
        final pending = fixture.flow.requestProfile(
          (name) async => writes.add(name),
        );
        await tester.pumpWidget(
          WidgetHelpers.testable(child: ProfileSetupPage(flow: fixture.flow)),
        );
        await tester.pumpAndSettle();
        final configuration = await presented.future;
        final name = List.filled(count, grapheme).join();
        final response = Completer<Map<dynamic, dynamic>>();
        messenger.handlePlatformMessage(
          channel.name,
          const StandardMethodCodec().encodeMethodCall(
            MethodCall('save', {
              'session': configuration['session'],
              'name': name,
            }),
          ),
          (data) => response.complete(
            const StandardMethodCodec().decodeEnvelope(data!) as Map,
          ),
        );
        expect(await response.future, {'saved': count == 100});
        expect(writes, count == 100 ? [name] : isEmpty);
        dismissed.complete(count == 100 ? 'saved' : 'skip');
        await tester.pumpAndSettle();
        await pending;
        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pumpAndSettle();
      }
    }
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));
}

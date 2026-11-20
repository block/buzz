import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/ios_create_channel_sheet.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets(
    'native channel creation retains its session through failure and retry',
    (tester) async {
      const bridge = MethodChannel('buzz/create_channel_sheet');
      final messenger = tester.binding.defaultBinaryMessenger;
      var dismissed = Completer<void>();
      Map<Object?, Object?>? configuration;
      messenger.setMockMethodCallHandler(bridge, (call) async {
        configuration = Map<Object?, Object?>.from(call.arguments as Map);
        await dismissed.future;
        return null;
      });
      addTearDown(() => messenger.setMockMethodCallHandler(bridge, null));
      final payloads = <Map<String, Object?>>[];
      final created = Channel(
        id: 'channel',
        name: 'Test',
        channelType: 'stream',
        visibility: 'private',
        description: 'Details',
        createdBy: 'owner',
        createdAt: DateTime(2026),
        memberCount: 1,
      );
      Channel? result;
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: Builder(
              builder: (context) => TextButton(
                onPressed: () async {
                  result = await IosCreateChannelSheet.present(
                    context: context,
                    onCreate:
                        ({
                          required name,
                          required description,
                          required visibility,
                          required ttlSeconds,
                        }) async {
                          payloads.add({
                            'name': name,
                            'description': description,
                            'visibility': visibility,
                            'ttl': ttlSeconds,
                          });
                          if (payloads.length == 1) {
                            throw StateError('Try again');
                          }
                          return created;
                        },
                  );
                },
                child: const Text('Open'),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open'));
      await tester.pump();
      final session = configuration!['session'];
      Future<Object?> create(String visibility, int? ttl) async {
        final response = Completer<Object?>();
        await messenger.handlePlatformMessage(
          bridge.name,
          const StandardMethodCodec().encodeMethodCall(
            MethodCall('create', {
              'session': session,
              'name': 'Test',
              'description': 'Details',
              'visibility': visibility,
              'ttlSeconds': ttl,
            }),
          ),
          (bytes) => response.complete(
            const StandardMethodCodec().decodeEnvelope(bytes!),
          ),
        );
        return response.future;
      }

      expect(
        await create('open', null),
        containsPair('error', contains('Try again')),
      );
      expect(result, isNull);
      expect(configuration!['session'], session);
      expect(await create('private', 3600), {'success': true});
      expect(payloads.last, {
        'name': 'Test',
        'description': 'Details',
        'visibility': 'private',
        'ttl': 3600,
      });
      expect(result, isNull); // Navigation waits until UIKit has dismissed.
      dismissed.complete();
      await tester.pumpAndSettle();
      expect(result, same(created));
      dismissed = Completer<void>();
      await tester.tap(find.text('Open'));
      await tester.pump();
      dismissed.complete(); // Native X or swipe dismissal returns no channel.
      await tester.pumpAndSettle();
      expect(result, isNull);
      expect(payloads, hasLength(2));
    },
  );
}

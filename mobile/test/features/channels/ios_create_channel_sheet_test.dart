import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/ios_create_channel_sheet.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('a failed channel create is described without internal error text', () {
    final cases = <Object, String>{
      const RelayEventRejectedException(
        'invalid: an open channel named "general" already exists',
      ): 'An open channel named "general" already exists',
      RelayNotConnectedError(): 'Couldn’t create channel. No connection.',
      RelayAckTimeoutError(
        'Event 371d27aa not acknowledged within 0:00:08.000000',
        const Duration(seconds: 8),
      ): "Not confirmed. The relay didn't respond.",
      RelayAckLostError(StateError('socket closed')):
          'Not confirmed. Connection lost.',
      // Anything else keeps its own text, minus the relay's machine prefix.
      Exception('restricted: not allowed to create channels'):
          'Not allowed to create channels',
    };
    for (final MapEntry(key: error, value: message) in cases.entries) {
      expect(createChannelErrorMessage(error), message, reason: '$error');
    }
  });

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
                            // What relay_session throws for a taken name.
                            throw const RelayEventRejectedException(
                              'invalid: an open channel named "Test" '
                              'already exists',
                            );
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

      expect(await create('open', null), {
        'error': 'An open channel named "Test" already exists',
      });
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

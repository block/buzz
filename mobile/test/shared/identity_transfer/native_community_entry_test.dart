import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/identity_transfer/native_community_entry.dart';
import 'package:buzz/shared/hosted/device_identity_source.dart';
import 'package:nostr/nostr.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  const channel = MethodChannel('buzz/community_entry');
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
  late Completer<bool> dismissed;
  late Completer<Map<dynamic, dynamic>> presented;
  setUp(() {
    dismissed = Completer<bool>();
    presented = Completer<Map<dynamic, dynamic>>();
    messenger.setMockMethodCallHandler(channel, (call) async {
      presented.complete(call.arguments as Map);
      return dismissed.future;
    });
  });
  tearDown(() => messenger.setMockMethodCallHandler(channel, null));

  Future<Map<dynamic, dynamic>?> action(
    String method,
    Map<String, dynamic> args,
  ) async {
    final result = Completer<Map<dynamic, dynamic>?>();
    messenger.handlePlatformMessage(
      channel.name,
      const StandardMethodCodec().encodeMethodCall(MethodCall(method, args)),
      (data) => result.complete(
        const StandardMethodCodec().decodeEnvelope(data!) as Map?,
      ),
    );
    return result.future;
  }

  for (final code in [
    'presentation_failed',
    'invalid_arguments',
    'already_presented',
  ]) {
    test('native $code preserves the presentation contract', () async {
      messenger.setMockMethodCallHandler(channel, (_) async {
        throw PlatformException(code: code);
      });
      final result = NativeCommunityEntry.show(
        name: 'Preview',
        dark: false,
        primaryArgb: 0xFF007AFF,
        choices: const [],
        connect: (_, _, _, _) async => fail('Must not connect'),
      );
      if (code == 'presentation_failed') {
        expect(await result, isNull);
        // A failed presentation must not leave the singleton busy.
        messenger.setMockMethodCallHandler(channel, (_) async => true);
        expect(
          await NativeCommunityEntry.show(
            name: 'Preview',
            dark: false,
            primaryArgb: 0xFF007AFF,
            choices: const [],
            connect: (_, _, _, _) async {},
          ),
          isTrue,
        );
      } else {
        await expectLater(
          result,
          throwsA(isA<PlatformException>().having((e) => e.code, 'code', code)),
        );
      }
    });
  }

  test(
    'invite confirmation delegates claiming and preserves retry errors',
    () async {
      var claims = 0;
      final shown = NativeCommunityEntry.show(
        name: 'preview.example.com',
        dark: false,
        primaryArgb: 0xFF000000,
        choices: const [],
        connect: (_, _, _, _) async => fail('Must use invite claiming'),
        confirmInvite: () async =>
            ++claims == 1 ? {'error': 'Try again'} : {'joined': true},
      );
      final config = await presented.future;
      expect(config['invite'], true);
      final args = <String, dynamic>{'session': config['session']};
      expect((await action('join', args))?['error'], 'Try again');
      expect((await action('join', args))?['joined'], true);
      dismissed.complete(true);
      expect(await shown, true);
    },
  );

  test(
    'native import requires preview and confirmation before connecting',
    () async {
      final keys = Keys('1'.padLeft(64, '0'));
      var joins = 0;
      final shown = NativeCommunityEntry.show(
        name: 'Preview',
        dark: false,
        primaryArgb: 0xFF007AFF,
        choices: [],
        connect: (_, nsec, create, current) async {
          expect(nsec, keys.nsec);
          expect(create, false);
          expect(current(), true);
          joins++;
        },
      );
      final config = await presented.future;
      final args = <String, dynamic>{
        'session': config['session'],
        'nsec': keys.nsec,
      };
      expect((await action('join', args))?['error'], isNotNull);
      expect(joins, 0);
      expect((await action('preview', args))?['npub'], startsWith('npub1'));
      expect(joins, 0);
      expect((await action('join', args))?['joined'], true);
      expect(joins, 1);
      dismissed.complete(true);
      expect(await shown, true);
    },
  );

  test(
    'returning to choices fences an in-flight join and allows retry',
    () async {
      final pending = Completer<void>();
      final began = Completer<void>();
      var calls = 0;
      var staleWasCurrent = true;
      final shown = NativeCommunityEntry.show(
        name: 'Preview',
        dark: false,
        primaryArgb: 0xFF007AFF,
        choices: const [DeviceIdentityChoice('pubkey', 'Saved')],
        connect: (_, _, _, current) async {
          calls++;
          if (calls == 1) {
            began.complete();
            await pending.future;
            staleWasCurrent = current();
          }
        },
      );
      final config = await presented.future;
      final args = <String, dynamic>{
        'session': config['session'],
        'pubkey': 'pubkey',
      };
      final first = action('join', args);
      await began.future;
      await action('cancel', args);
      pending.complete();
      expect((await first)?['cancelled'], true);
      expect(staleWasCurrent, false);
      expect((await action('join', args))?['joined'], true);
      expect(calls, 2);
      dismissed.complete(true);
      expect(await shown, true);
    },
  );
}

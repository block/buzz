import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/push/android_push_authority.dart';
import 'package:buzz/shared/push/android_push_gateway.dart';
import 'package:buzz/shared/push/android_push_keystore.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import '../../helpers/android_push_fakes.dart';
import '../../helpers/recording_diagnostics.dart';

const _channel = MethodChannel('buzz/push_registration');

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUp(() {
    fcmDeviceToken.value = null;
    fcmRegistrationError.value = null;
    pushUnavailableReason.value = null;
    pushEndpointGrants.value = const [];
    pushEndpointGrantError.value = null;
    installBuzzPushMethodHandler();
  });

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_channel, null);
    debugDefaultTargetPlatformOverride = null;
    debugAndroidPushAuthority = null;
  });

  Future<void> native(String method, Map<String, Object?> arguments) =>
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .handlePlatformMessage(
            _channel.name,
            _channel.codec.encodeMethodCall(MethodCall(method, arguments)),
            (_) {},
          );

  test('asks the native side to register and to refresh the token', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    final methods = <String>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_channel, (call) async {
          methods.add(call.method);
          return null;
        });

    await startBuzzPushRegistration();
    await refreshBuzzPushToken();

    expect(methods, ['startRegistration', 'refreshToken']);
  });

  // Each row: the state before, native token events, then what Settings sees.
  // "Unavailable" is reserved for a phone that cannot get a token at all or
  // cannot attest; a token fetch failing while a token is held is transient.
  const attest = 'The phone cannot attest its key.';
  for (final row
      in <
        ({
          String name,
          String? token,
          String? shown,
          List<(String, String)> events,
          String? reason,
        })
      >[
        (
          name: 'a token clears an earlier registration failure',
          token: null,
          shown: null,
          events: [('fail', 'no network'), ('token', 'fcm-token')],
          reason: null,
        ),
        (
          name: 'a failure with no token shows unavailable',
          token: null,
          shown: null,
          events: [('fail', 'no network')],
          reason: 'no network',
        ),
        (
          name: 'a token does not clear an attestation failure',
          token: null,
          shown: attest,
          events: [('token', 'fcm-token')],
          reason: attest,
        ),
        (
          name: 'a failure while a token is held shows nothing',
          token: 'fcm-token',
          shown: null,
          events: [('fail', 'SERVICE_NOT_AVAILABLE')],
          reason: null,
        ),
        (
          name: 'a failure then a token leave an attestation failure',
          token: null,
          shown: attest,
          events: [('fail', 'no network'), ('token', 'fcm-token')],
          reason: attest,
        ),
        (
          name: 'two different failures then a token clear the row',
          token: null,
          shown: null,
          events: [
            ('fail', 'no network'),
            ('fail', 'SERVICE_NOT_AVAILABLE'),
            ('token', 'fcm-token'),
          ],
          reason: null,
        ),
      ]) {
    test(row.name, () async {
      fcmDeviceToken.value = row.token;
      pushUnavailableReason.value = row.shown;

      for (final (kind, value) in row.events) {
        if (kind == 'fail') {
          await native('fcmRegistrationFailed', {'message': value});
          expect(fcmRegistrationError.value, value);
        } else {
          await native('fcmTokenChanged', {'token': value});
          expect(fcmDeviceToken.value, value);
          expect(fcmRegistrationError.value, isNull);
        }
      }

      expect(pushUnavailableReason.value, row.reason);
    });
  }

  test('enrolls through the Dart authority and exposes its grants', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    var now = 1_750_000_000;
    final gateway = FakePushGateway(nowSeconds: () => now);
    debugAndroidPushAuthority = AndroidPushAuthority(
      keystore: FakeAndroidKeystore(),
      gateway: fakeGatewayClient(gateway),
      store: _MemoryStore(),
      now: () => DateTime.fromMillisecondsSinceEpoch(now * 1000),
      sleep: (_) async {},
    );
    fcmDeviceToken.value = 'fcm-token';
    pushUnavailableReason.value = 'stale';

    final grant = (await enrollBuzzPush(
      'wss://relay.example/',
      'https://gateway.example/',
      relayOrigin: 'wss://relay.example',
      relayPubkey: 'a' * 64,
      desiredRelays: () => {'wss://relay.example'},
    ))!;

    expect(grant.relayOrigin, 'wss://relay.example');
    expect(grant.relayPubkey, 'a' * 64);
    expect(pushUnavailableReason.value, isNull);
    expect(pushEndpointGrants.value.single.endpointGrant, grant.endpointGrant);
    expect(
      (await readBuzzPushEndpointGrants()).single.installationId,
      grant.installationId,
    );
  });

  test('a phone that cannot attest reports push as unavailable', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    var now = 1_750_000_000;
    debugAndroidPushAuthority = AndroidPushAuthority(
      keystore: FakeAndroidKeystore()..attests = false,
      gateway: fakeGatewayClient(FakePushGateway(nowSeconds: () => now)),
      store: _MemoryStore(),
      now: () => DateTime.fromMillisecondsSinceEpoch(now * 1000),
      sleep: (_) async {},
    );
    fcmDeviceToken.value = 'fcm-token';

    await expectLater(
      enrollBuzzPush(
        'wss://relay.example/',
        'https://gateway.example/',
        relayOrigin: 'wss://relay.example',
        relayPubkey: 'a' * 64,
        desiredRelays: () => {'wss://relay.example'},
      ),
      throwsA(isA<AndroidPushAttestationUnavailable>()),
    );

    expect(pushUnavailableReason.value, isNotNull);
  });

  test('only a gateway reply the app cannot read is reported', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    fcmDeviceToken.value = 'fcm-token';
    final diagnostics = RecordingDiagnostics();

    Future<void> enroll() => enrollBuzzPush(
      'wss://relay.example/',
      'https://gateway.example/',
      relayOrigin: 'wss://relay.example',
      relayPubkey: 'a' * 64,
      desiredRelays: () => {'wss://relay.example'},
      diagnostics: diagnostics,
    ).then((_) {});

    for (final (reply, reported) in [
      // A refusal and gateway trouble are routine.
      (http.Response(jsonEncode({'error': 'unavailable'}), 503), false),
      // Even when the refusal's body is too large to read.
      (http.Response('x' * 65537, 503), false),
      // A challenge with no fields, and a body that is not JSON, are not.
      (http.Response(jsonEncode({}), 200), true),
      (http.Response('<html>', 200), true),
      (http.Response('x' * 65537, 200), true),
    ]) {
      diagnostics.errors.clear();
      debugAndroidPushAuthority = AndroidPushAuthority(
        keystore: FakeAndroidKeystore(),
        gateway: AndroidPushGatewayClient(
          baseUrl: Uri.parse('https://push.example'),
          client: MockClient((_) async => reply),
        ),
        store: _MemoryStore(),
        sleep: (_) async {},
      );
      await expectLater(enroll(), throwsA(anything));
      expect(diagnostics.errors, [
        if (reported)
          (
            AndroidPushGatewayResponseInvalid,
            DiagnosticOperation.pushGatewayResponse,
          ),
      ], reason: '${reply.statusCode} ${reply.body.length} bytes');
    }
  });

  test('a gateway call that times out is aborted, not abandoned', () async {
    final aborted = Completer<void>();
    final gateway = AndroidPushGatewayClient(
      baseUrl: Uri.parse('https://push.example'),
      timeout: const Duration(milliseconds: 10),
      client: _HangingClient(aborted),
    );

    await expectLater(
      gateway.post('/v1/installations/challenges', {'v': 1}),
      throwsA(isA<TimeoutException>()),
    );
    await aborted.future.timeout(const Duration(seconds: 1));
  });

  test('enrolling without a token fails loudly', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;

    await expectLater(
      enrollBuzzPush(
        'wss://relay.example/',
        'https://gateway.example/',
        relayOrigin: 'wss://relay.example',
        relayPubkey: 'a' * 64,
        desiredRelays: () => {'wss://relay.example'},
      ),
      throwsStateError,
    );
  });
}

class _MemoryStore implements AndroidPushStateStore {
  String? value;

  @override
  Future<String?> read() async => value;

  @override
  Future<void> write(String? next) async => value = next;
}

/// Never answers, and completes [aborted] when the request's abort trigger
/// fires.
class _HangingClient extends http.BaseClient {
  _HangingClient(this.aborted);

  final Completer<void> aborted;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) {
    final trigger = (request as http.Abortable).abortTrigger;
    if (trigger == null) throw StateError('the request is not abortable');
    trigger.then((_) => aborted.complete());
    return Completer<http.StreamedResponse>().future;
  }
}

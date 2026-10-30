import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/push/push_relay_capability_provider.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test(
    'valid capability starts independent permission and APNs registration',
    () async {
      var requests = 0;

      await startBuzzPushRegistrationIfCapable(
        _descriptor,
        startRegistration: () async {
          requests += 1;
        },
      );

      expect(requests, 1);
    },
  );

  test(
    'missing capability cannot start permission or APNs registration',
    () async {
      var requests = 0;

      await startBuzzPushRegistrationIfCapable(
        null,
        startRegistration: () async {
          requests += 1;
        },
      );

      expect(requests, 0);
    },
  );

  for (final failure in <Object>[
    const FormatException('malformed descriptor'),
    StateError('relay unreachable'),
  ]) {
    test('$failure keeps capability inactive without registration', () async {
      final descriptor = await discoverBuzzPushRelayCapability(
        'https://relay.example',
        fetchDescriptor: (_) async => throw failure,
      );
      var requests = 0;

      await startBuzzPushRegistrationIfCapable(
        descriptor,
        startRegistration: () async {
          requests += 1;
        },
      );

      expect(descriptor, isNull);
      expect(requests, 0);
    });
  }
}

const _descriptor = BuzzPushLeaseDescriptor(
  origin: 'wss://relay.example',
  executorKeyId: 'relay-v1',
  executorPubkey:
      'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  transport: 'apns',
  maxLeaseTtlSeconds: 3600,
  maxContentLength: 4096,
  maxPlaintextLength: 4096,
  maxEndpointLength: 2048,
  maxStringLength: 512,
);

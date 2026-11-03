import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/relay/relay_auth_discovery.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

void main() {
  const capable = {
    'limitation': {'federated_identity': true},
    'federated_identity': {
      'core': 'client-attached',
      'assertion_freshness': {'class': 'offline-jwt'},
    },
  };

  test('capability alone never establishes a host requirement', () {
    for (final info in [<String, Object?>{}, capable]) {
      expect(
        parseRelayAuthDiscovery(info).requirement,
        RelayAuthRequirement.legacy,
      );
      expect(
        parseRelayAuthDiscovery({
          ...info,
          'requires_nip_fi': false,
        }).requirement,
        RelayAuthRequirement.optional,
      );
    }
    expect(
      parseRelayAuthDiscovery({
        ...capable,
        'requires_nip_fi': true,
      }).requirement,
      RelayAuthRequirement.required,
    );
    for (final info in [
      null,
      [],
      {'requires_nip_fi': null},
      {'requires_nip_fi': 'true'},
      {'requires_nip_fi': 1},
    ]) {
      expect(() => parseRelayAuthDiscovery(info), throwsFormatException);
    }
  });

  test('required policy survives unsupported or absent descriptors', () {
    for (final (descriptor, compatible) in [
      (capable, true),
      (<String, Object?>{}, false),
      ({...capable, 'limitation': null}, false),
      ({...capable, 'federated_identity': null}, false),
      for (final freshness in ['current-status', 'unknown-class', null])
        (
          {
            ...capable,
            'federated_identity': {
              'core': 'client-attached',
              'assertion_freshness': {'class': freshness},
            },
          },
          false,
        ),
      (
        {
          ...capable,
          'federated_identity': {'core': 'another-protocol'},
        },
        false,
      ),
    ]) {
      expect(
        parseRelayAuthDiscovery({...descriptor, 'requires_nip_fi': true}),
        (requirement: RelayAuthRequirement.required, compatible: compatible),
      );
    }
  });

  test(
    'a new fetch observes this host upgrading without leaking credentials',
    () async {
      var upgraded = false;
      final client = MockClient((request) async {
        expect(request.headers['Accept'], 'application/nostr+json');
        expect(request.headers.containsKey('Authorization'), isFalse);
        expect(
          request.headers.containsKey('Nostr-Federated-Identity'),
          isFalse,
        );
        expect(request.followRedirects, isFalse);
        return http.Response(
          jsonEncode({
            ...capable,
            'requires_nip_fi':
                request.url.host == 'required.example' && upgraded,
          }),
          200,
        );
      });
      addTearDown(client.close);
      final target = Uri.parse('https://required.example/info');
      expect(
        (await fetchRelayAuthDiscovery(client, target)).requirement,
        RelayAuthRequirement.optional,
      );
      upgraded = true;
      expect(
        (await fetchRelayAuthDiscovery(client, target)).requirement,
        RelayAuthRequirement.required,
      );
      expect(
        (await fetchRelayAuthDiscovery(
          client,
          Uri.parse('https://free.example/info'),
        )).requirement,
        RelayAuthRequirement.optional,
      );
    },
  );

  test(
    'HTTP, network and malformed responses cannot become upgrade verdicts',
    () async {
      for (final status in [302, 401, 403, 404, 503]) {
        final client = MockClient((_) async => http.Response('', status));
        addTearDown(client.close);
        await expectLater(
          fetchRelayAuthDiscovery(
            client,
            Uri.parse('https://community.example/info'),
          ),
          throwsA(
            isA<RelayAuthDiscoveryFailure>().having(
              (error) => error.status,
              'status',
              status,
            ),
          ),
        );
      }
      final unavailable = MockClient(
        (_) async => throw http.ClientException('offline'),
      );
      addTearDown(unavailable.close);
      await expectLater(
        fetchRelayAuthDiscovery(
          unavailable,
          Uri.parse('https://community.example/info'),
        ),
        throwsA(isA<http.ClientException>()),
      );
      for (final body in [
        'invalid',
        '[]',
        '{"requires_nip_fi":null}',
        ' ' * (256 * 1024 + 1),
      ]) {
        final client = MockClient((_) async => http.Response(body, 200));
        addTearDown(client.close);
        await expectLater(
          fetchRelayAuthDiscovery(
            client,
            Uri.parse('https://community.example/info'),
          ),
          throwsFormatException,
        );
      }
    },
  );

  test('a stalled body times out and aborts its request', () async {
    final client = _StalledDiscoveryClient();
    addTearDown(client.close);
    await expectLater(
      fetchRelayAuthDiscovery(
        client,
        Uri.parse('https://community.example/info'),
        deadline: Duration.zero,
      ),
      throwsA(isA<TimeoutException>()),
    );
    await client.aborted;
  });

  test('discovery refuses credential-bearing or non-discovery URLs', () async {
    final client = MockClient((_) async {
      fail('Invalid discovery URL reached the network');
    });
    addTearDown(client.close);
    for (final url in [
      'https://user:password@community.example/info',
      'https://community.example/info?token=secret',
      'https://community.example/info#secret',
      'https://community.example/query',
      'wss://community.example',
    ]) {
      await expectLater(
        fetchRelayAuthDiscovery(client, Uri.parse(url)),
        throwsFormatException,
      );
    }
  });
}

class _StalledDiscoveryClient extends http.BaseClient {
  late final Future<void> aborted;
  final body = StreamController<List<int>>();

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    aborted = (request as http.AbortableRequest).abortTrigger!;
    return http.StreamedResponse(body.stream, 200);
  }

  @override
  void close() {
    unawaited(body.close());
    super.close();
  }
}

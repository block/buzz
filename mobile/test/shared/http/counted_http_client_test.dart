import 'dart:async';
import 'dart:io';

import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/diagnostics/diagnostic_member_binding.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/http/app_http_client.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../helpers/recording_diagnostics.dart';

void main() {
  late RecordingDiagnostics diagnostics;

  setUp(() {
    diagnostics = RecordingDiagnostics();
    appHttpDiagnostics = diagnostics;
    addTearDown(() => appHttpDiagnostics = const NoopDiagnosticReporter());
  });

  /// One request through the app's shared client, answered by [handler].
  Future<Object?> send(
    String url,
    Future<http.Response> Function(http.Request) handler, {
    String method = 'GET',
    HttpTraffic traffic = HttpTraffic.api,
  }) async {
    final client = http.runWithClient(
      () => newAppHttpClient(traffic),
      () => MockClient(handler),
    );
    try {
      return (await client.send(
        http.Request(method, Uri.parse(url)),
      )).statusCode;
    } catch (error) {
      return error;
    } finally {
      client.close();
    }
  }

  Future<http.Response> status(int code) async => http.Response('', code);

  test('every response is counted by its status', () async {
    final cases = <int, NetworkFailure?>{
      200: null,
      204: null,
      302: null,
      401: NetworkFailure.authRefused,
      403: NetworkFailure.authRefused,
      404: NetworkFailure.clientError,
      408: NetworkFailure.timeout,
      429: NetworkFailure.rateLimited,
      500: NetworkFailure.serverError,
      503: NetworkFailure.serverError,
    };
    for (final code in cases.keys) {
      expect(
        await send('https://relay.example.org/', (_) => status(code)),
        code,
      );
    }
    expect(
      [for (final event in diagnostics.events) event.failure],
      [...cases.values],
    );
    for (final event in diagnostics.events) {
      expect(event.backend, 'relay.example.org');
      expect(
        event.outcome,
        event.failure == null ? NetworkOutcome.success : NetworkOutcome.failure,
      );
      expect(event.duration, isNotNull);
    }
    // Only failures are worth a place on the trail.
    expect(diagnostics.breadcrumbs, hasLength(cases.values.nonNulls.length));
    expect(diagnostics.errors, isEmpty);
  });

  test('requests are counted under a fixed endpoint, never a path', () async {
    final cases = <(String, String, HttpTraffic), NetworkEndpoint>{
      ('POST', 'https://relay.example.org/query', HttpTraffic.api):
          NetworkEndpoint.relayHttpQuery,
      ('POST', 'https://relay.example.org/profiles/search', HttpTraffic.api):
          NetworkEndpoint.relayHttpQuery,
      ('GET', 'https://relay.example.org/', HttpTraffic.api):
          NetworkEndpoint.relayInfo,
      ('GET', 'https://relay.example.org', HttpTraffic.api):
          NetworkEndpoint.relayInfo,
      ('POST', 'https://relay.example.org/api/invites/claim', HttpTraffic.api):
          NetworkEndpoint.communityAdmission,
      ('GET', 'https://relay.example.org/api/join-policy', HttpTraffic.api):
          NetworkEndpoint.communityAdmission,
      ('GET', 'https://relay.example.org/invite/abc123', HttpTraffic.api):
          NetworkEndpoint.communityAdmission,
      (
        'POST',
        'https://account.example.org/api/goose/v1/auth/me',
        HttpTraffic.api,
      ): NetworkEndpoint.builderlabAccount,
      ('GET', 'https://cdn.example.org/card-7/agent.json', HttpTraffic.api):
          NetworkEndpoint.otherHttp,
      ('GET', 'https://relay.example.org/media/abc.jpg', HttpTraffic.media):
          NetworkEndpoint.mediaDownload,
      ('POST', 'https://relay.example.org/upload', HttpTraffic.media):
          NetworkEndpoint.mediaUpload,
    };
    for (final MapEntry(key: (method, url, traffic)) in cases.entries) {
      await send(url, (_) => status(200), method: method, traffic: traffic);
    }
    expect(
      [for (final event in diagnostics.events) event.endpoint],
      [...cases.values],
    );
    expect(
      {for (final event in diagnostics.events) event.backend},
      {'relay.example.org', 'account.example.org', 'cdn.example.org'},
    );
  });

  test('a path or query string never leaves as a backend', () async {
    await send(
      'https://relay.example.org/channels/secret-room?token=abc#frag',
      (_) => status(200),
    );
    final event = diagnostics.events.single;
    expect(event.backend, 'relay.example.org');
    expect(event.endpoint, NetworkEndpoint.otherHttp);
  });

  test(
    'a transport failure is counted by kind and still reaches the caller',
    () async {
      final cases = <Object, NetworkFailure>{
        TimeoutException('slow'): NetworkFailure.timeout,
        const SocketException('Connection refused'): NetworkFailure.unreachable,
        const SocketException('Failed host lookup: relay.example.org'):
            NetworkFailure.unreachable,
        const SocketException(
          'Connection failed',
          osError: OSError('Network is unreachable', 101),
        ): NetworkFailure.offline,
        const SocketException(
          'Connection failed',
          osError: OSError('Operation timed out', 60),
        ): NetworkFailure.timeout,
        const HandshakeException('CERTIFICATE_VERIFY_FAILED'):
            NetworkFailure.insecure,
        http.ClientException(
          'Connection closed before full header was received',
        ): NetworkFailure.disconnected,
        http.ClientException('something else entirely'): NetworkFailure.other,
      };
      final results = <Object?>[];
      for (final error in cases.keys) {
        results.add(
          await send('https://relay.example.org/', (_) => throw error),
        );
      }
      expect(results, cases.keys.toList());
      expect(diagnostics.events.map((event) => event.failure), [
        ...cases.values,
      ]);
      expect(diagnostics.events.every((e) => e.duration != null), isTrue);
      expect(diagnostics.breadcrumbs, hasLength(cases.length));
      // Routine failures are counted and never reported as errors.
      expect(diagnostics.errors, isEmpty);
    },
  );

  test('a request cancelled by closing its client is not a failure', () async {
    final hung = Completer<http.Response>();
    final client = http.runWithClient(
      newAppHttpClient,
      () => MockClient((_) => hung.future),
    );
    final request = client.get(Uri.parse('https://relay.example.org/'));
    final settled = request.then<void>((_) {}, onError: (_) {});
    await pumpEventQueue();
    client.close();
    hung.completeError(http.RequestAbortedException());
    await settled;

    expect(diagnostics.events, isEmpty);
  });

  test('an abort with no deadline behind it is not counted', () async {
    expect(
      await send(
        'https://relay.example.org/',
        (_) => throw http.RequestAbortedException(),
      ),
      isA<http.RequestAbortedException>(),
    );

    expect(diagnostics.events, isEmpty);
    expect(diagnostics.breadcrumbs, isEmpty);
  });

  test('a deadline that passes before headers counts one timeout, even when '
      'its owner closes the client', () async {
    final hung = Completer<http.Response>();
    final client = http.runWithClient(
      newAppHttpClient,
      () => MockClient((_) => hung.future),
    );
    final request = http.Request(
      'POST',
      Uri.parse('https://relay.example.org/query'),
    );
    final settled = client.send(request).then<void>((_) {}, onError: (_) {});
    await pumpEventQueue();
    noteHttpDeadline(request);
    client.close();
    hung.completeError(http.ClientException('Client closed'));
    await settled;

    expect(diagnostics.events.map((event) => event.failure), [
      NetworkFailure.timeout,
    ]);
    expect(diagnostics.events.single.endpoint, NetworkEndpoint.relayHttpQuery);
    expect(diagnostics.breadcrumbs, hasLength(1));
  });

  test('a request that finishes after another member signs in is not '
      'counted for them', () async {
    diagnostics.setMember('a' * 64);
    final answer = Completer<http.Response>();
    final pending = send(
      'https://relay.example.org/query',
      (_) => answer.future,
    );
    await pumpEventQueue();
    diagnostics.setMember('b' * 64);
    answer.complete(http.Response('', 503));
    await pending;

    expect(diagnostics.counts, isEmpty);
    expect(diagnostics.breadcrumbs, isEmpty);

    await send('https://relay.example.org/query', (_) => status(503));
    expect(diagnostics.counts.single.member, 'b' * 64);
  });

  test('a deadline that passes after headers leaves the success', () async {
    final client = http.runWithClient(
      newAppHttpClient,
      () => MockClient((_) async => http.Response('', 200)),
    );
    addTearDown(client.close);
    final request = http.Request(
      'GET',
      Uri.parse('https://relay.example.org/'),
    );
    await client.send(request);
    noteHttpDeadline(request);

    expect(diagnostics.events.map((event) => event.outcome), [
      NetworkOutcome.success,
    ]);
  });

  test(
    'a counting call that fails does not fail or delay the request',
    () async {
      appHttpDiagnostics = ThrowingDiagnostics();

      expect(await send('https://relay.example.org/', (_) => status(200)), 200);
      expect(await send('https://relay.example.org/', (_) => status(500)), 500);
      expect(
        await send(
          'https://relay.example.org/',
          (_) => throw TimeoutException('x'),
        ),
        isA<TimeoutException>(),
      );
    },
  );

  test(
    'signing out and switching members changes who counts are about',
    () async {
      final container = ProviderContainer(
        overrides: [
          diagnosticReporterProvider.overrideWithValue(diagnostics),
          activeCommunityProvider.overrideWith((ref) async => null),
        ],
      );
      addTearDown(container.dispose);
      container.listen(diagnosticMemberProvider, (_, _) {});
      final relay = container.read(relayConfigProvider.notifier);
      String nsec(String digit) =>
          nostr.Nip19.encode(prefix: nostr.Nip19Prefix.nsec, data: digit * 64);

      Future<void> request() =>
          send('https://relay.example.org/', (_) => status(200));

      relay.update(baseUrl: 'https://relay.example.org', nsec: nsec('1'));
      container.read(diagnosticMemberProvider);
      await request();
      relay.update(baseUrl: 'https://relay.example.org', nsec: nsec('2'));
      container.read(diagnosticMemberProvider);
      await request();
      relay.update(baseUrl: 'https://relay.example.org');
      container.read(diagnosticMemberProvider);
      await request();

      final members = [for (final count in diagnostics.counts) count.member];
      expect(members[0], isNotNull);
      expect(members[1], isNotNull);
      expect(members[0], isNot(members[1]));
      expect(members[2], isNull);
    },
  );
}

import 'dart:async';
import 'dart:convert';
import 'dart:io' show HttpDate;

import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

/// Issuance through the real account client and relay auth, with the phone's
/// wall clock and monotonic clock driven independently of the issuer's.
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final keys = nostr.Keys(
    '0000000000000000000000000000000000000000000000000000000000000001',
  );
  final organization = Uri.parse('https://block.example.com');
  const relayHost = 'buzz.block.example.com';
  const otherHost = 'other.block.example.com';
  final relay = RelayConfig(baseUrl: 'https://$relayHost', nsec: keys.nsec);
  final other = RelayConfig(baseUrl: 'https://$otherHost', nsec: keys.nsec);
  final expiry = DateTime.now().toUtc().add(const Duration(days: 1));
  const server = 1800000000;

  // The phone's clocks.
  late int wall;
  late Duration elapsed;
  // What the issuer answers for each assertion request, in order; the last
  // one repeats.
  late List<Future<http.StreamedResponse> Function()> answers;
  late int issued;
  late Object? identity;

  setUp(() async {
    FlutterSecureStorage.setMockInitialValues({});
    await const FlutterSecureStorage().write(
      key: 'buzz_hosted_session_block_example_com_v1',
      value: jsonEncode({
        'origin': organization.toString(),
        'credential': 'org-secret',
        'subject': 'member',
        'expires_at': expiry.toIso8601String(),
      }),
    );
    wall = server;
    elapsed = const Duration(seconds: 1000);
    issued = 0;
    answers = [];
    identity = {'pubkey_hex': keys.public};
  });

  String jws(Map<String, Object?> claims) {
    String segment(Object value) =>
        base64Url.encode(utf8.encode(jsonEncode(value))).replaceAll('=', '');
    return '${segment({'alg': 'ES256'})}.${segment(claims)}.c2ln';
  }

  http.StreamedResponse json(Object body, [int status = 200]) =>
      http.StreamedResponse(
        Stream.value(utf8.encode(jsonEncode(body))),
        status,
        headers: {'content-type': 'application/json'},
      );

  /// An assertion the issuer signs at [iat] for [lifetime] seconds, after
  /// the reply spends [delay] in flight.
  Future<http.StreamedResponse> Function() token(
    String label, {
    int iat = server,
    int lifetime = 180,
    int delay = 0,
    bool withIat = true,
  }) => () async {
    elapsed += Duration(seconds: delay);
    wall += delay;
    return json({
      'assertion': jws({
        if (withIat) 'iat': iat,
        'exp': iat + lifetime,
        'jti': label,
      }),
      'nostr_pubkey': keys.public,
      'expires_at': iat + lifetime,
    });
  };

  /// A 403 whose `Date` reads [serverNow] and whose body then does [body].
  Future<http.StreamedResponse> Function() refusal(
    DateTime serverNow,
    Stream<List<int>> Function() body,
  ) =>
      () async => http.StreamedResponse(
        body(),
        403,
        headers: {'date': HttpDate.format(serverNow)},
      );

  BuilderlabAccountClient account() => BuilderlabAccountClient(
    accountOrigin: organization,
    httpClient: MockClient.streaming((request, _) async {
      return switch (request.url.path) {
        '/api/goose/v1/auth/me' => json({
          'subject': 'member',
          'expires_at': expiry.toIso8601String(),
        }),
        '/api/goose/v1/buzz/nostr-identities/current' => json({
          'identity': identity,
        }),
        '/api/goose/v1/buzz/identity/assertions' =>
          await answers[issued++ < answers.length
              ? issued - 1
              : answers.length - 1](),
        _ => json({}, 404),
      };
    }),
  );

  HostedRelayAuth relayAuth(BuilderlabAccountClient organizationAccount) =>
      HostedRelayAuth(
        BuilderlabAccountClient(
          httpClient: MockClient((_) async => fail('hosted account unused')),
        ),
        organization: organizationAccount,
        organizationRelay: (_) => true,
        enabled: true,
        now: () => DateTime.fromMillisecondsSinceEpoch(wall * 1000),
        elapsed: () => elapsed,
      );

  String? sent(Map<String, String> headers) =>
      headers['Nostr-Federated-Identity'];

  Map<String, Object?> claimsOf(String? header) => jsonDecode(
    utf8.decode(base64Url.decode(base64Url.normalize(header!.split('.')[1]))),
  );

  group('lifetime', () {
    for (final (name, phoneOffset, refreshAfter) in [
      // 180 s from the request start, refreshed 30 s early.
      ('a phone 120 s slow', -120, 150),
      // `exp` leaves 60 s on a phone 120 s fast; refreshed 30 s early.
      ('a phone 120 s fast', 120, 30),
    ]) {
      test('$name keeps the token only for its real life', () async {
        wall = server + phoneOffset;
        answers = [token('a')];
        final auth = relayAuth(account());
        final header = sent(await auth.headers(relay));
        expect(claimsOf(header)['jti'], 'a');
        elapsed += Duration(seconds: refreshAfter - 1);
        expect(sent(auth.cachedHeaders(relay)!), header);
        elapsed += const Duration(seconds: 1);
        expect(auth.cachedHeaders(relay), isNull);
      });
    }

    test(
      'a reply delayed past the token life is refused and reissued',
      () async {
        // 10 s token, 20 s in flight: never sent. The second one is.
        wall = server - 100;
        answers = [token('late', lifetime: 10, delay: 20), token('good')];
        final auth = relayAuth(account());
        expect(claimsOf(sent(await auth.headers(relay)))['jti'], 'good');
        expect(issued, 2);
        expect(claimsOf(sent(auth.cachedHeaders(relay)!))['jti'], 'good');
      },
    );

    test('reissuance is bounded when no token has life left', () async {
      wall = server - 100;
      answers = [token('late', lifetime: 10, delay: 20)];
      final auth = relayAuth(account());
      await expectLater(
        auth.headers(relay),
        throwsA(AccountFailure.unavailable),
      );
      expect(issued, 3);
      expect(auth.cachedHeaders(relay), isNull);
    });

    test('a short token after a slow reply keeps only what is left', () async {
      // 30 s token, 10 s in flight: 20 s left, refreshed at half of it.
      answers = [token('short', lifetime: 30, delay: 10)];
      final auth = relayAuth(account());
      await auth.headers(relay);
      elapsed += const Duration(seconds: 9);
      expect(auth.cachedHeaders(relay), isNotNull);
      elapsed += const Duration(seconds: 1);
      expect(auth.cachedHeaders(relay), isNull);
      answers = [token('renewed')];
      expect(claimsOf(sent(await auth.headers(relay)))['jti'], 'renewed');
    });

    test('an assertion without iat is refused', () async {
      answers = [token('no-iat', withIat: false)];
      final auth = relayAuth(account());
      await expectLater(
        auth.headers(relay),
        throwsA(AccountFailure.invalidResponse),
      );
      expect(auth.cachedHeaders(relay), isNull);
    });
  });

  group('refusal clock hint', () {
    final fast = DateTime.now().subtract(const Duration(minutes: 10));

    for (final (name, body) in [
      (
        'a delayed body',
        () => Stream.fromFuture(
          Future.delayed(
            const Duration(seconds: 3),
            () => utf8.encode('{"error":"invalid_proof"}'),
          ),
        ),
      ),
      (
        'a lost body',
        () => Stream<List<int>>.error(const SocketExceptionStub()),
      ),
      ('an oversized body', () => Stream.value(List.filled(1 << 20, 0x20))),
    ]) {
      test('a 403 with $name stays denied and keeps its Date hint', () async {
        answers = [refusal(fast, body)];
        final auth = relayAuth(account());
        await expectLater(auth.headers(relay), throwsA(AccountFailure.denied));
        expect(auth.failure, AccountFailure.denied);
        expect(auth.failureClockHint, 10);
      });
    }

    test('the offset is read when the headers arrive, not the body', () async {
      // 114 s off at the headers gives no hint; read 3 s later, at the body,
      // it would be over the 115 s threshold.
      answers = [
        refusal(
          DateTime.now().subtract(const Duration(seconds: 114)),
          () => Stream.fromFuture(
            Future.delayed(
              const Duration(seconds: 3),
              () => utf8.encode('{"error":"invalid_proof"}'),
            ),
          ),
        ),
      ];
      final auth = relayAuth(account());
      await expectLater(auth.headers(relay), throwsA(AccountFailure.denied));
      expect(auth.failureClockHint, isNull);
    });

    test('a phone clock behind the server is hinted too', () async {
      answers = [
        refusal(
          DateTime.now().add(const Duration(minutes: 3)),
          () => Stream.value(utf8.encode('{}')),
        ),
      ];
      final auth = relayAuth(account());
      await expectLater(auth.headers(relay), throwsA(AccountFailure.denied));
      expect(auth.failureClockHint, 3);
    });

    test('a normal offset or no usable Date gives no hint', () async {
      for (final date in [HttpDate.format(DateTime.now()), 'garbage', null]) {
        answers = [
          () async => http.StreamedResponse(
            Stream.value(utf8.encode('{}')),
            403,
            headers: {'date': ?date},
          ),
        ];
        final auth = relayAuth(account());
        await expectLater(auth.headers(relay), throwsA(AccountFailure.denied));
        expect(auth.failureClockHint, isNull, reason: '$date');
      }
    });

    test('the hint follows its refusal through its lifecycle', () async {
      final organizationAccount = account();
      final auth = relayAuth(organizationAccount);
      answers = [refusal(fast, () => Stream.value(utf8.encode('{}')))];
      await expectLater(auth.headers(relay), throwsA(AccountFailure.denied));
      expect(auth.failureClockHint, 10);

      // Another community's issuance leaves it.
      issued = 0;
      answers = [token('other')];
      await auth.headers(other);
      expect(auth.failureClockHint, 10);

      // A successful issuance for the refused community clears it.
      issued = 0;
      answers = [token('ok')];
      await auth.headers(relay);
      expect(auth.failureClockHint, isNull);

      // So does a session change.
      issued = 0;
      answers = [refusal(fast, () => Stream.value(utf8.encode('{}')))];
      auth.rejected(relay, sent(auth.cachedHeaders(relay)!));
      await expectLater(auth.headers(relay), throwsA(AccountFailure.denied));
      expect(auth.failureClockHint, 10);
      await organizationAccount.clearLocalSession();
      expect(auth.failureClockHint, isNull);
    });

    test('an old refusal never explains a later local denial', () async {
      final auth = relayAuth(account());
      answers = [refusal(fast, () => Stream.value(utf8.encode('{}')))];
      await expectLater(auth.headers(relay), throwsA(AccountFailure.denied));
      expect(auth.failureClockHint, 10);
      // The key no longer matches the account's identity: denied locally,
      // with no new 403, so no hint.
      identity = {'pubkey_hex': List.filled(64, 'f').join()};
      await expectLater(auth.headers(relay), throwsA(AccountFailure.denied));
      expect(auth.failure, AccountFailure.denied);
      expect(auth.failureClockHint, isNull);
    });
  });
}

class SocketExceptionStub implements Exception {
  const SocketExceptionStub();
}

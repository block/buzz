import 'dart:convert';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/shared/relay/media_auth.dart';
import 'package:buzz/shared/relay/relay_session.dart';
import 'package:buzz/shared/crypto/event_signer.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/hosted/web_connect_binding.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  try {
    await runProbe();
  } catch (error) {
    debugPrint('SIGNING_PROBE FAILED: $error');
    rethrow;
  }
}

Future<void> runProbe() async {
  if (!Platform.isIOS) throw StateError('This probe requires an iOS Runner');
  final results = <String>[];
  final key = '0' * 63 + '3';
  final nsec = nostr.Nip19.encode(prefix: nostr.Nip19Prefix.nsec, data: key);
  final cold = Stopwatch()..start();
  final first = signEvent(
    kind: 9,
    content: 'iOS native signing 🐝',
    secretKey: key,
  );
  results.add('cold complete event: ${cold.elapsedMicroseconds} us');
  if (!first.isValid()) throw StateError('Native signature rejected by Dart');
  for (var i = 0; i < 100; i++) {
    signEvent(kind: 9, content: '', secretKey: key);
  }
  for (final count in [1, 8, 32, 64]) {
    final samples = <int>[];
    for (var round = 0; round < 100; round++) {
      final watch = Stopwatch()..start();
      for (var i = 0; i < count; i++) {
        signEvent(
          kind: 24242,
          content: '',
          secretKey: key,
          tags: [
            ['t', 'get'],
            ['expiration', '1800000060'],
          ],
        );
      }
      samples.add(watch.elapsedMicroseconds);
    }
    samples.sort();
    results.add(
      'burst $count: p50 ${samples[50]} p95 ${samples[95]} max ${samples.last} us',
    );
  }
  final media = MediaGetAuthService(
    baseUrl: 'https://relay.example',
    nsec: nsec,
  );
  final watch = Stopwatch()..start();
  final proofs = <Map<String, String>>[];
  for (var i = 0; i < 64; i++) {
    proofs.add(media.headersFor('https://relay.example/media/$i'));
  }
  results.add('64 complete media headers: ${watch.elapsedMicroseconds} us');
  for (final proof in proofs) {
    final encoded = proof['Authorization']!.split(' ').last;
    final event = nostr.Event.fromJson(utf8.decode(base64Decode(encoded)));
    if (!event.isValid()) throw StateError('Invalid media proof');
  }
  if ((media.headersFor('https://foreign.example/media/x')).isNotEmpty) {
    throw StateError('Foreign host received credentials');
  }
  results.add('media signatures and foreign-host isolation: PASS');
  final queryWatch = Stopwatch()..start();
  final headers = <String>[];
  for (var i = 0; i < 32; i++) {
    headers.add(
      buildNip98AuthHeader(
        nsec: nsec,
        url: 'https://relay.example/query',
        method: 'POST',
        bodyBytes: utf8.encode(
          jsonEncode({
            'kinds': [9],
            '#h': ['channel-$i'],
            'limit': 50,
          }),
        ),
      ),
    );
  }
  results.add(
    '32 complete NIP98 headers: ${queryWatch.elapsedMicroseconds} us',
  );
  for (final header in headers) {
    final event = nostr.Event.fromJson(
      utf8.decode(base64Decode(header.split(' ').last)),
    );
    if (!event.isValid()) throw StateError('Invalid query proof');
  }
  results.add('query signatures: PASS');
  final keys = nostr.Keys(key);
  final binding = parseNostrBindDeepLink(
    Uri(
      scheme: 'buzz',
      host: 'nostr-bind',
      queryParameters: {
        'challenge_id': '12345678-1234-1234-1234-123456789012',
        'nonce': 'a' * 43,
        'verification_code': '123456',
        'audience': 'buzz:nostr-identity',
        'action': 'bind_nostr_identity',
        'protocol': 'buzz-nostr-identity',
        'version': '1',
        'origin': HostedEnvironment.origin.toString(),
        'expires_at': '2099-01-01T00:00:00Z',
        'return': 'browser_fragment_v1',
        'callback_url': HostedEnvironment.origin.resolve('/buzz').toString(),
      },
    ),
  )!;
  var bindingVerified = false;
  final returned = await WebConnectBindingService().complete(
    request: binding,
    subject: 'synthetic-account',
    expectedPubkey: keys.public,
    withKeys: <T>(Future<T> Function(nostr.Keys) operation) => operation(keys),
    verify:
        ({
          required subject,
          required challengeId,
          required nonce,
          required signedPayload,
        }) async {
          final event = nostr.Event.fromJson(signedPayload);
          if (event.kind != 24243 ||
              event.pubkey != keys.public ||
              !event.isValid()) {
            throw StateError('Invalid hosted binding proof');
          }
          bindingVerified = true;
          return keys.public;
        },
  );
  if (!bindingVerified ||
      returned != HostedEnvironment.origin.resolve('/buzz')) {
    throw StateError('Hosted binding did not verify before browser return');
  }
  results.add('hosted binding signature and browser return: PASS');
  for (final result in results) {
    debugPrint('SIGNING_PROBE $result');
  }
  debugPrint('SIGNING_PROBE COMPLETE');
  runApp(
    MaterialApp(
      home: Scaffold(body: SafeArea(child: Text(results.join('\n')))),
    ),
  );
}

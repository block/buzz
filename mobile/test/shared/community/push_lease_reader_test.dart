import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:buzz/shared/relay/relay_provider.dart' show RelayConfig;
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  late HttpServer server;
  late Community community;
  late Community other;

  setUp(() async {
    // The test binding answers every HttpClient request with a 400, but the
    // reader fetches the relay's push descriptor from a real local server.
    final overrides = HttpOverrides.current;
    HttpOverrides.global = null;
    addTearDown(() => HttpOverrides.global = overrides);
    server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    addTearDown(() => server.close(force: true));
    final origin = 'ws://127.0.0.1:${server.port}';
    server.listen(
      (request) => request.response
        ..headers.contentType = ContentType('application', 'nostr+json')
        ..write(jsonEncode(_descriptorJson(origin)))
        ..close(),
    );
    community = Community.create(
      name: 'Here',
      relayUrl: 'ws://127.0.0.1:${server.port}',
      nsec: nostr.Keys.generate().nsec,
    ).copyWith(pushLeaseInstallationId: 'd' * 32);
    other = Community.create(
      name: 'Elsewhere',
      relayUrl: 'https://relay.example.org',
      nsec: nostr.Keys.generate().nsec,
    );
  });

  RelayConfig relayOf(Community c) =>
      RelayConfig(baseUrl: c.relayUrl, nsec: c.nsec);

  test('reads its own lease from the writer, and finds none', () async {
    final sent = <Map<String, dynamic>>[];
    final lease = await readCommunityPushLease(
      community,
      query: (filters) async {
        sent.addAll(filters.map((f) => f.toJson()));
        return const <NostrEvent>[];
      },
      currentRelay: () => relayOf(community),
    );

    expect(lease, isNull);
    expect(sent, [
      {
        'kinds': [buzzPushLeaseKind],
        'authors': [
          nostr.Keys(nostr.Nip19.decode(payload: community.nsec!).data).public,
        ],
        '#d': ['d' * 32],
        'limit': 1,
        'consistency': 'strong',
      },
    ]);
  });

  // A switch to another community while the read waits would settle this
  // community from the other's relay, so the read fails instead.
  for (final (name, switchDuringQuery) in [
    ('the descriptor fetch', false),
    ('the query', true),
  ]) {
    test('a community switch during $name fails the read', () async {
      var current = switchDuringQuery ? community : other;
      var queried = false;
      final read = readCommunityPushLease(
        community,
        query: (filters) async {
          queried = true;
          current = other;
          return const <NostrEvent>[];
        },
        currentRelay: () => relayOf(current),
      );

      await expectLater(read, throwsStateError);
      expect(queried, switchDuringQuery);
    });
  }
}

Map<String, dynamic> _descriptorJson(String origin) => {
  'supported_extensions': ['nip-er', 'buzz-push-v1'],
  'push': {
    'origin': origin,
    'keys': [
      {
        'id': 'relay-v1',
        'pubkey': nostr.Keys('2' * 64).public,
        'current': true,
      },
    ],
    'push_kinds': [9, 40002, 45001, 45003],
    'h_grammar': 'uuid-lowercase',
    'class_support': {
      'apns': ['default'],
    },
    'limitation': {
      'max_lease_ttl': 2592000,
      'max_leases_per_pubkey': 16,
      'max_subscriptions_per_lease': 16,
      'max_kinds': 16,
      'max_authors': 20,
      'max_h': 50,
      'max_tag_values': 20,
      'max_ignore': 8,
      'max_content_len': 65536,
      'max_plaintext_len': 32768,
      'max_endpoint_len': 4096,
      'max_string_len': 512,
    },
  },
};

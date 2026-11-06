import 'dart:convert';
import 'dart:typed_data';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/me/me_archived_agents.dart';
import 'package:buzz/shared/me/me_agent_inventory.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';

class _Relay extends RelaySessionNotifier {
  final List<List<NostrEvent>> pages;
  List<NostrEvent>? inventory;
  List<List<NostrEvent>>? inventoryPages;
  int inventoryReads = 0;
  final requestedTags = <Map<String, dynamic>>[];
  final requestedAuthors = <List<String>?>[];
  _Relay(this.pages, {this.inventory});
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);
  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final filter = filters.single;
    requestedTags.add(filter.toJson());
    if (filter.kinds.single == 30177) {
      return inventoryPages?[inventoryReads++] ?? inventory ?? const [];
    }
    expect(filter.kinds, [0]);
    expect(
      filter.authors,
      isNotEmpty,
      reason: 'Never scan all community profiles',
    );
    requestedAuthors.add(filter.authors);
    return pages.expand((p) => p).toList();
  }
}

void main() {
  final owner = nostr.Keys.generate();
  final other = nostr.Keys.generate();
  final agent = nostr.Keys.generate();
  NostrEvent profile(
    nostr.Keys signer, {
    nostr.Keys? ownedBy,
    int time = 1,
    String conditions = '',
    Object? content,
  }) {
    final digest = SHA256Digest()
        .process(
          Uint8List.fromList(
            utf8.encode('nostr:agent-auth:${signer.public}:$conditions'),
          ),
        )
        .map((v) => v.toRadixString(16).padLeft(2, '0'))
        .join();
    return NostrEvent.fromJson(
      nostr.Event.from(
        kind: 0,
        content: jsonEncode(content ?? {'display_name': 'My assistant'}),
        secretKey: signer.secret,
        createdAt: time,
        tags: [
          if (ownedBy != null)
            [
              'auth',
              ownedBy.public,
              conditions,
              nostr.Schnorr.sign(secretKey: ownedBy.secret, message: digest),
            ],
        ],
      ).toMap(),
    );
  }

  NostrEvent listing(
    String pubkey, {
    String name = 'Saved agent',
    int time = 1,
    String? content,
  }) => NostrEvent.fromJson(
    nostr.Event.from(
      kind: 30177,
      content:
          content ??
          jsonEncode({
            'name': name,
            'parallelism': 1,
            'respond_to': 'owner-only',
          }),
      secretKey: owner.secret,
      createdAt: time,
      tags: [
        ['d', pubkey],
      ],
    ).toMap(),
  );

  ProviderContainer setup(_Relay relay, {Set<String> archived = const {}}) {
    relay.inventory ??= [listing(agent.public)];
    final sync = MeSync(
      relay,
      RelayConfig(
        baseUrl: 'https://relay.test',
        nsec: nostr.Nip19.encode(
          prefix: nostr.Nip19Prefix.nsec,
          data: owner.secret,
        ),
      ),
    );
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => relay),
        meSyncProvider.overrideWithValue(sync),
        meArchivedAgentsProvider.overrideWith((ref) async => archived),
      ],
    );
    addTearDown(container.dispose);
    return container;
  }

  test(
    'lists current inventory with verified ownership, name and artwork',
    () async {
      final valid = profile(
        agent,
        ownedBy: owner,
        content: {
          'display_name': 'Profile name',
          'picture': 'https://example.com/agent.png',
        },
      );
      final forged = valid.toJson()..['pubkey'] = other.public;
      final container = setup(
        _Relay([
          [
            profile(owner),
            profile(nostr.Keys.generate(), ownedBy: other),
            NostrEvent.fromJson(forged),
            valid,
          ],
        ]),
      );
      final choices = await container.read(meAssistantsProvider.future);
      expect(choices.map((a) => a.pubkey), [agent.public]);
      expect(choices.single.displayName, 'Saved agent');
      expect(choices.single.avatarUrl, 'https://example.com/agent.png');
    },
  );
  for (final malformed in [7, true, 'wrong', <String, Object>{}, <Object>[]]) {
    test(
      'malformed optional profile fields do not poison directory: $malformed',
      () async {
        final broken = nostr.Keys.generate();
        final container = setup(
          _Relay([
            [
              profile(
                broken,
                ownedBy: owner,
                content: {
                  for (final field in [
                    'display_name',
                    'name',
                    'respond_to',
                    'respond_to_allowlist',
                    'channel_ids',
                  ])
                    field: malformed,
                },
              ),
              profile(agent, ownedBy: owner),
            ],
          ]),
        );
        final choices = await container.read(meAssistantsProvider.future);
        expect(
          choices.singleWhere((a) => a.pubkey == agent.public).displayName,
          'Saved agent',
        );
        expect(
          await isOwnedMeAssistant(
            container.read(meSyncProvider),
            agent.public,
          ),
          isTrue,
        );
      },
    );
  }
  test(
    'inventory pagination advances and rejects a repeated full page',
    () async {
      final first = listing(agent.public, time: 2);
      final last = listing(other.public, time: 1);
      final page = List.filled(200, first);
      final relay = _Relay([])
        ..inventoryPages = [
          page,
          [last],
        ];
      final sync = setup(relay).read(meSyncProvider);
      expect(
        (await loadMeAgentInventory(sync)).keys,
        unorderedEquals([agent.public, other.public]),
      );
      expect(relay.requestedTags.last['until'], 2);
      expect(relay.requestedTags.last['before_id'], first.id);
      expect(relay.requestedTags.first['authors'], [owner.public]);
      relay.inventoryReads = 0;
      relay.inventoryPages = [page, page];
      await expectLater(loadMeAgentInventory(sync), throwsStateError);
      expect(relay.inventoryReads, 2);
    },
  );

  test('latest profile removes obsolete ownership', () async {
    final container = setup(
      _Relay([
        [profile(agent, ownedBy: owner), profile(agent, time: 2)],
      ]),
    );
    expect(await container.read(meAssistantsProvider.future), isEmpty);
  });
  test(
    'omits historical, removed and archived agents without losing ownership checks',
    () async {
      final archivedAgent = nostr.Keys.generate();
      final removed = nostr.Keys.generate();
      final historical = nostr.Keys.generate();
      final relay = _Relay(
        [
          [
            for (final key in [agent, archivedAgent, removed, historical])
              profile(key, ownedBy: owner),
          ],
        ],
        inventory: [
          listing(agent.public),
          for (final field in ['id', 'sig'])
            NostrEvent.fromJson(
              listing(agent.public, time: 3, content: '{}').toJson()
                ..[field] = '0' * (field == 'id' ? 64 : 128),
            ),
          listing(archivedAgent.public),
          listing(removed.public),
          listing(removed.public, time: 2, content: '{}'),
        ],
      );
      final container = setup(relay, archived: {archivedAgent.public});
      expect(
        (await container.read(
          meAssistantsProvider.future,
        )).map((a) => a.pubkey),
        [agent.public],
      );
      expect(
        relay.requestedAuthors.single,
        unorderedEquals([agent.public, archivedAgent.public]),
      );
      expect(
        await isOwnedMeAssistant(
          container.read(meSyncProvider),
          historical.public,
        ),
        isTrue,
      );
    },
  );
  test(
    'empty inventory is authoritative and does not scan historical profiles',
    () async {
      final relay = _Relay([
        [profile(agent, ownedBy: owner)],
      ], inventory: []);
      expect(await setup(relay).read(meAssistantsProvider.future), isEmpty);
      expect(relay.requestedAuthors, isEmpty);
    },
  );
  const policy = {
    'name': 'Saved agent',
    'parallelism': 1,
    'respond_to': 'owner-only',
  };
  for (final (label, content) in [
    ('unsupported respond_to', {...policy, 'respond_to': 'nobody'}),
    ('missing respond_to', {'name': 'Agent', 'parallelism': 1}),
    ('missing parallelism', {'name': 'Agent', 'respond_to': 'anyone'}),
    ('negative parallelism', {...policy, 'parallelism': -1}),
    ('fractional parallelism', {...policy, 'parallelism': 1.5}),
    ('oversized parallelism', {...policy, 'parallelism': 0x100000000}),
    ('non-list allowlist', {...policy, 'respond_to_allowlist': 'x'}),
    (
      'non-string allowlist entry',
      {
        ...policy,
        'respond_to': 'allowlist',
        'respond_to_allowlist': [7],
      },
    ),
    ('null allowlist', {...policy, 'respond_to_allowlist': null}),
    ('non-string model', {...policy, 'model': 7}),
    ('non-object content', ['Agent']),
  ]) {
    test('an unsupported latest policy fails closed: $label', () async {
      final relay = _Relay(
        [
          [profile(agent, ownedBy: owner)],
        ],
        inventory: [
          listing(agent.public, time: 2, content: jsonEncode(content)),
          listing(agent.public),
        ],
      );
      expect(await setup(relay).read(meAssistantsProvider.future), isEmpty);
    });
  }
  test('accepts every supported policy shape', () async {
    final shapes = [
      policy,
      {...policy, 'respond_to': 'anyone', 'parallelism': 0},
      {
        ...policy,
        'respond_to': 'allowlist',
        'respond_to_allowlist': [other.public],
        'persona_id': 'scout',
        'model': null,
        'future_field': true,
      },
    ];
    final agents = [for (final _ in shapes) nostr.Keys.generate()];
    final relay = _Relay(
      [
        [for (final key in agents) profile(key, ownedBy: owner)],
      ],
      inventory: [
        for (var i = 0; i < shapes.length; i++)
          listing(agents[i].public, content: jsonEncode(shapes[i])),
      ],
    );
    expect(
      (await setup(
        relay,
      ).read(meAssistantsProvider.future)).map((a) => a.pubkey),
      unorderedEquals(agents.map((key) => key.public)),
    );
  });
  test('inventory rejects records outside owner scope', () async {
    final foreign = NostrEvent.fromJson(
      nostr.Event.from(
        kind: 30177,
        content: '{"name":"Foreign"}',
        secretKey: other.secret,
        tags: [
          ['d', agent.public],
        ],
      ).toMap(),
    );
    final container = setup(_Relay([], inventory: [foreign]));
    await expectLater(
      loadMeAgentInventory(container.read(meSyncProvider)),
      throwsStateError,
    );
  });
  for (final conditions in ['kind=1', 'created_at>1', 'created_at<1']) {
    test(
      'discovery and send revalidation reject inapplicable $conditions',
      () async {
        final relay = _Relay([
          [profile(agent, ownedBy: owner, conditions: conditions)],
        ]);
        final container = setup(relay);
        expect(await container.read(meAssistantsProvider.future), isEmpty);
        expect(
          await isOwnedMeAssistant(
            container.read(meSyncProvider),
            agent.public,
          ),
          isFalse,
        );
      },
    );
  }
  test('creation revalidates only the chosen assistant', () async {
    final relay = _Relay([
      [profile(agent, ownedBy: owner)],
    ]);
    final container = setup(relay);
    expect(
      await isOwnedMeAssistant(container.read(meSyncProvider), agent.public),
      isTrue,
    );
    expect(relay.requestedAuthors, [
      [agent.public],
    ]);
  });
  test(
    'creation rejects revoked ownership and unrelated returned profiles',
    () async {
      final relay = _Relay([
        [
          profile(agent, ownedBy: owner),
          profile(agent, time: 2),
          profile(other, ownedBy: owner),
        ],
      ]);
      final container = setup(relay);
      expect(
        await isOwnedMeAssistant(container.read(meSyncProvider), agent.public),
        isFalse,
      );
    },
  );
}

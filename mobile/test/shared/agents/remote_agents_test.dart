import 'package:buzz/features/profile/presence_cache_provider.dart';
import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/crypto/nip_oa.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/storage/secure_storage.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/remote_agents_fixture.dart';

final _scope = 'remote-agents:${HostedEnvironment.origin.host}:$fakeSubject';

Community _community({String? pubkey}) => Community.create(
  name: 'Test',
  relayUrl: 'wss://test.${HostedEnvironment.communitySuffix}',
  pubkey: pubkey ?? fakeOwner.public,
  nsec: fakeOwner.nsec,
);

Future<RelayFakeController> _controller(FakeRegistry registry) async {
  await AccountIdentityStore().forSubject(fakeSubject).create(fakeOwner.nsec);
  final controller = RelayFakeController(registry);
  addTearDown(() {
    controller.dispose();
    controller.relayAuth.dispose();
    registry.dispose();
  });
  return controller;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() {
    FlutterSecureStorage.setMockInitialValues({});
    SharedPreferences.setMockInitialValues({});
  });

  test('remote agents follow the build-time Builderlab gate', () {
    expect(supportsRemoteAgents(_community()), HostedEnvironment.enabled);
    expect(supportsRemoteAgents(null), isFalse);
    final selfHosted = Community.create(
      name: 'Self hosted',
      relayUrl: 'wss://relay.example.com',
      pubkey: fakeOwner.public,
    );
    expect(supportsRemoteAgents(selfHosted), isFalse);
  });

  test('listing requires a signed-in account bound to the saved key', () async {
    final registry = FakeRegistry(bound: fakeOwner.public, signedIn: false);
    final controller = await _controller(registry);
    registry.listed = [
      {
        'agent_pubkey': fakeAgent,
        'agent_name': 'Scout',
        'status': 'BEEKEEPER_AGENT_STATUS_ACTIVE',
      },
    ];

    expect(await controller.run(controller.refresh), isTrue);
    expect(registry.calls, isEmpty);
    expect(controller.activeAgents, isEmpty);
    expect(controller.status, contains('Sign in'));

    registry
      ..signedIn = true
      ..bound = nostr.Keys('0' * 63 + '3').public;
    expect(await controller.run(controller.refresh), isFalse);
    expect(registry.calls, isEmpty);
    expect(controller.status, 'Buzz identity: mismatch.');

    registry.bound = fakeOwner.public;
    expect(await controller.run(controller.refresh), isTrue);
    expect(registry.calls, ['list-agents']);
    expect(controller.owner, fakeOwner.public);
    expect(controller.activeAgents, {fakeAgent});

    registry.signedIn = false;
    expect(controller.activeAgents, isEmpty);
  });

  test(
    'failed setup resumes the same identity; instructions stay private',
    () async {
      final registry = FakeRegistry(bound: fakeOwner.public)
        ..failInstructions = true;
      final controller = await _controller(registry);
      final community = _community();

      expect(
        await controller.create(community, 'Scout', instructions: 'x' * 20001),
        isFalse,
      );
      expect(
        await controller.create(_community(pubkey: 'f' * 64), 'Scout'),
        isFalse,
      );
      expect(registry.calls, isEmpty);
      expect(controller.pending, isNull);

      expect(
        await controller.create(
          community,
          'Scout',
          instructions: 'Reply concisely.',
        ),
        isFalse,
      );
      expect(registry.calls, [
        'register-agent',
        'attest-agent',
        'update-agent',
      ]);
      expect(controller.pending?['pubkey'], fakeAgent);
      final prefs = await SharedPreferences.getInstance();
      expect(prefs.getString('$_scope:pending'), isNotNull);
      expect(
        prefs.getString('$_scope:pending'),
        isNot(contains('Reply concisely.')),
      );

      // A fresh controller restores the pending setup from storage.
      final resumed = RelayFakeController(registry);
      addTearDown(() {
        resumed.dispose();
        resumed.relayAuth.dispose();
      });
      registry
        ..failInstructions = false
        ..calls.clear();
      expect(
        await resumed.create(community, 'Other', instructions: 'Other'),
        isTrue,
      );
      expect(registry.calls, [
        'attest-agent',
        'update-agent',
        'add-buzz-subscription',
        'publish-policy',
        'list-agents',
      ]);
      expect(
        registry.bodies['/v3/beekeeper/update-agent']?['agent_instructions'],
        'Reply concisely.',
      );
      expect(
        registry.bodies['/v3/beekeeper/attest-agent']?['community_url'],
        community.relayUrl,
      );
      expect(
        registry.bodies['/v3/beekeeper/add-buzz-subscription']?['community'],
        Uri.parse(community.relayUrl).host,
      );
      expect(resumed.pending, isNull);
      expect(prefs.getString('$_scope:pending'), isNull);
    },
  );

  test(
    'deletion requires confirmed registration absence; disabled stays retryable',
    () async {
      final registry = FakeRegistry(bound: fakeOwner.public);
      final controller = await _controller(registry);
      registry.listed = [
        {'agent_pubkey': fakeAgent, 'agent_id': 'remote-id'},
      ];
      await controller.run(controller.refresh);
      registry.calls.clear();
      var registrationExists = true;
      var current = true;
      Future<bool> remove() => controller.deleteAgent(
        pubkey: fakeAgent,
        community: _community(),
        isCurrent: () => current,
        fetch: (filter) async {
          registry.calls.add('read-registration');
          expect(filter.kinds, [30177]);
          expect(filter.authors, [fakeOwner.public]);
          expect(filter.tags, {
            '#d': [fakeAgent],
          });
          return registrationExists
              ? [
                  NostrEvent(
                    id: 'registration',
                    pubkey: fakeOwner.public,
                    createdAt: 1,
                    kind: 30177,
                    tags: [
                      ['d', fakeAgent],
                    ],
                    content: '{}',
                    sig: '',
                  ),
                ]
              : [];
        },
      );

      controller.rejectRemoval = true;
      expect(await remove(), isFalse);
      expect(registry.calls, ['remove-registration']);
      registry.calls.clear();
      controller.rejectRemoval = false;
      expect(await remove(), isFalse);
      expect(registry.calls, ['remove-registration', 'read-registration']);
      registry
        ..calls.clear()
        ..deleteStatus = 2;
      registrationExists = false;
      expect(await remove(), isFalse);
      expect(controller.status, contains('disabled'));
      expect(controller.agents, hasLength(1));

      // The relay registration is gone, but the listing keeps a retry row.
      registry.listed = [];
      await controller.run(controller.refresh);
      expect(controller.agents.single['agent_id'], 'remote-id');

      registry.calls.clear();
      current = false;
      expect(await remove(), isFalse);
      expect(registry.calls, isEmpty);
      current = true;
      registry.deleteStatus = 'DELETE_AGENT_STATUS_NOT_FOUND';
      expect(await remove(), isTrue);
      expect(registry.calls.last, 'delete-agent');
      expect(controller.agents, isEmpty);
      expect(controller.deletedAgents, {fakeAgent});
      await controller.run(controller.refresh);
      expect(controller.agents, isEmpty);
    },
  );

  test('a setup without a server identity can be discarded', () async {
    final registry = FakeRegistry(bound: fakeOwner.public)
      ..rejectRegistration = true;
    final controller = await _controller(registry);
    final community = _community();
    final prefs = await SharedPreferences.getInstance();

    expect(
      await controller.create(community, 'Scout', instructions: 'Private.'),
      isFalse,
    );
    expect(controller.status, 'Registration rejected.');
    final request = controller.pending!['request'];
    expect(prefs.getString('$_scope:pending'), isNotNull);

    expect(await controller.discardPending(), isTrue);
    expect(controller.pending, isNull);
    expect(prefs.getString('$_scope:pending'), isNull);
    expect(
      await accountSecureStorage.read(key: '$_scope:instructions:$request'),
      isNull,
    );

    // Once registered, the setup holds a real agent and can only resume.
    registry
      ..rejectRegistration = false
      ..failInstructions = true;
    expect(
      await controller.create(community, 'Scout', instructions: 'Private.'),
      isFalse,
    );
    expect(controller.pending?['pubkey'], fakeAgent);
    expect(await controller.discardPending(), isFalse);
    expect(controller.pending?['pubkey'], fakeAgent);
  });

  test('another account never sees the previous registry', () async {
    final registry = FakeRegistry(bound: fakeOwner.public);
    final controller = await _controller(registry);
    registry.listed = [
      {
        'agent_pubkey': fakeAgent,
        'agent_name': 'Scout',
        'status': 'BEEKEEPER_AGENT_STATUS_ACTIVE',
      },
    ];
    await controller.run(controller.refresh);
    expect(controller.activeAgents, {fakeAgent});

    var notified = 0;
    controller.addListener(() => notified++);
    registry.subject = 'other-owner';
    expect(notified, 1);
    expect(controller.activeAgents, isEmpty);
    // The other account has no saved key, so its registry is never listed.
    expect(await controller.run(controller.refresh), isFalse);
    expect(controller.agents, isEmpty);
    expect(controller.owner, isNull);
  });

  test('a failed registry load retries on the next load', () async {
    final registry = FakeRegistry(bound: 'f' * 64);
    final controller = await _controller(registry);
    final relay = _community().relayUrl;
    await controller.load(relay);
    expect(controller.status, 'Buzz identity: mismatch.');
    registry.bound = fakeOwner.public;
    await controller.load(relay);
    expect(registry.calls, ['list-agents']);
    await controller.load(relay);
    expect(registry.calls, ['list-agents']);
  });

  test(
    'availability counts only agents registered in this community',
    () async {
      final other = nostr.Keys('0' * 63 + '3').public;
      final stranger = nostr.Keys('0' * 63 + '4');
      final elsewhere = nostr.Keys('0' * 63 + '5').public;
      final profiles = [
        NostrEvent(
          id: '1',
          pubkey: fakeAgent,
          createdAt: 1,
          kind: 0,
          tags: [signOwnerAuthorization(fakeOwner, fakeAgent)],
          content: '{}',
          sig: '',
        ),
        // Attested by someone else, so not this owner's agent.
        NostrEvent(
          id: '2',
          pubkey: other,
          createdAt: 1,
          kind: 0,
          tags: [signOwnerAuthorization(stranger, other)],
          content: '{}',
          sig: '',
        ),
      ];
      expect(
        await registeredCommunityAgents(
          {fakeAgent, other, elsewhere},
          fakeOwner.public,
          (filter) async {
            expect(filter.kinds, [0]);
            return profiles;
          },
        ),
        {fakeAgent},
      );
      expect(
        await registeredCommunityAgents(
          {fakeAgent},
          null,
          (_) async => profiles,
        ),
        isEmpty,
      );
    },
  );

  test('owner proof verifies for only the provisioned agent key', () {
    final other = nostr.Keys('0' * 63 + '3');
    final auth = signOwnerAuthorization(fakeOwner, fakeAgent);
    expect(verifiedOaOwnerPubkey([auth], fakeAgent), fakeOwner.public);
    expect(verifiedOaOwnerPubkey([auth], other.public), isNull);
    expect(
      () => signOwnerAuthorization(fakeOwner, fakeOwner.public),
      throwsStateError,
    );
  });

  test('disabled and missing statuses cannot count as activation', () {
    requireStatus({'status': 'ATTEST_AGENT_STATUS_ACTIVE'}, 'ACTIVE', 1);
    requireStatus({'status': 1}, 'ACTIVE', 1);
    for (final result in [
      {'status': 'ATTEST_AGENT_STATUS_FEATURE_DISABLED'},
      <String, dynamic>{},
    ]) {
      expect(() => requireStatus(result, 'ACTIVE', 1), throwsStateError);
    }
  });

  test(
    'remote availability shows Online without changing local presence',
    () async {
      final container = ProviderContainer(
        overrides: [
          presenceCacheProvider.overrideWith(_Presence.new),
          availableRemoteAgentsProvider.overrideWith((ref) async => {'remote'}),
        ],
      );
      addTearDown(container.dispose);
      await container.read(availableRemoteAgentsProvider.future);
      expect(container.read(presentedPresenceProvider), {
        'remote': 'online',
        'local': 'away',
      });
      expect(container.read(presenceCacheProvider)['remote'], 'offline');
    },
  );
}

class _Presence extends PresenceCacheNotifier {
  @override
  Map<String, String> build() => {'remote': 'offline', 'local': 'away'};
}

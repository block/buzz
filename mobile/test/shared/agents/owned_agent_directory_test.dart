import 'package:buzz/shared/agents/owned_agent_directory.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../helpers/remote_agents_fixture.dart';

NostrEvent registration(
  String agent, {
  String content = '{"name":"Agent"}',
  String? runtime = 'block.builderlab/builderlab',
  int createdAt = 1,
}) => signedAgentRegistration(
  agent,
  content: content,
  runtime: runtime,
  createdAt: createdAt,
);

class _Session extends RelaySessionNotifier {
  _Session(this.registrations, this.profiles);
  final List<NostrEvent> registrations;
  final List<NostrEvent> profiles;

  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);

  void connect() => state = const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final filter = filters.single;
    if (filter.kinds.contains(30177)) {
      return filter.extensions['page'] == 1 ? registrations : const [];
    }
    return profiles
        .where((profile) => filter.authors!.contains(profile.pubkey))
        .toList();
  }
}

class _RelayConfig extends RelayConfigNotifier {
  @override
  RelayConfig build() =>
      const RelayConfig(baseUrl: 'https://relay.example', nsec: null);
}

void main() {
  test('stays unknown, not empty, until the session connects', () async {
    final session = _Session(
      [registration(fakeAgent)],
      [signedAgentProfile(fakeAgentKeys, fakeOwner)],
    );
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(_RelayConfig.new),
        myPubkeyProvider.overrideWithValue(fakeOwner.public),
        relaySessionProvider.overrideWith(() => session),
      ],
    );
    addTearDown(container.dispose);
    final subscription = container.listen(
      ownedAgentDirectoryProvider,
      (_, _) {},
    );
    addTearDown(subscription.close);

    await Future<void>.delayed(Duration.zero);
    expect(container.read(ownedAgentDirectoryProvider).isLoading, isTrue);
    expect(container.read(ownedAgentDirectoryProvider).asData, isNull);

    session.connect();
    final found = await container.read(ownedAgentDirectoryProvider.future);
    expect(found.map((entry) => entry.pubkey), [fakeAgent]);
    final recipients = await container.read(
      ownedAgentRecipientsProvider.future,
    );
    expect(recipients.map((entry) => entry.pubkey), [fakeAgent]);

    session.registrations.clear();
    container.invalidate(ownedAgentInventoryProvider);
    await container.pump();
    expect(await container.read(ownedAgentDirectoryProvider.future), isEmpty);
    expect(await container.read(ownedAgentRecipientsProvider.future), isEmpty);
  });

  test('lists owned agents as desktop does, without runtime directory '
      'entries or account sign-in', () async {
    final desktop = nostr.Keys('0' * 63 + '3');
    final outsider = nostr.Keys('0' * 63 + '4');
    final oldApp = nostr.Keys('0' * 63 + '5');
    final reverted = nostr.Keys('0' * 63 + '6');
    final imported = nostr.Keys('0' * 63 + '7');
    final registrations = [
      registration(
        fakeAgent,
        content: '{"name":"Remote","model":"Claude Sonnet"}',
      ),
      registration(desktop.public, runtime: 'block.claude-code/claude-code'),
      registration(outsider.public),
      // An earlier Buzz wrote no runtime; desktop lists it once imported.
      registration(oldApp.public, runtime: null),
      registration(imported.public, runtime: null),
      registration(imported.public, createdAt: 2),
      // Only the newest registration counts.
      registration(reverted.public),
      registration(reverted.public, runtime: null, createdAt: 2),
    ];
    final profiles = [
      signedAgentProfile(
        fakeAgentKeys,
        fakeOwner,
        content: '{"name":"Remote"}',
      ),
      signedAgentProfile(desktop, fakeOwner, content: '{"name":"Desktop"}'),
      signedAgentProfile(outsider, desktop, content: '{"name":"Not owned"}'),
      for (final keys in [oldApp, reverted, imported])
        signedAgentProfile(keys, fakeOwner, content: '{"name":"Owned"}'),
    ];
    final found = await loadOwnedAgentDirectory(fakeOwner.public, (
      filter,
    ) async {
      if (filter.kinds.contains(30177)) {
        expect(filter.authors, [fakeOwner.public]);
        return registrations;
      }
      expect(filter.kinds, [0]);
      return profiles
          .where((profile) => filter.authors!.contains(profile.pubkey))
          .toList();
    });
    expect(found.first.model, 'Claude Sonnet');
    expect(found.first.runtime, 'block.builderlab/builderlab');
    expect(found[1].runtime, 'block.claude-code/claude-code');
    expect(found.map((entry) => entry.pubkey), [
      fakeAgent,
      desktop.public,
      imported.public,
    ]);
  });

  test('a relay that never ends pagination fails within the budget', () async {
    final template = registration(fakeAgent).toJson();
    var pages = 0;
    await expectLater(
      loadOwnedAgentDirectory(fakeOwner.public, (filter) async {
        pages++;
        // Fresh ids on every page; the loop must stop on its own budget.
        return [
          for (var i = 0; i < 100; i++)
            NostrEvent.fromJson({
              ...template,
              'id': (pages * 1000 + i).toRadixString(16).padLeft(64, '0'),
            }),
        ];
      }),
      throwsStateError,
    );
    expect(pages, ownedAgentDirectoryPageBudget);
  });

  test('forged runtime metadata cannot mark an agent as hosted', () async {
    final forged = NostrEvent.fromJson({
      ...registration(fakeAgent).toJson(),
      'sig': '0' * 128,
    });
    final found = await loadOwnedAgentDirectory(
      fakeOwner.public,
      (filter) async => filter.kinds.contains(30177)
          ? [forged]
          : [signedAgentProfile(fakeAgentKeys, fakeOwner)],
    );
    expect(found.single.runtime, isNull);
  });

  test('a registration whose signature fails keeps the agent without its '
      'model', () async {
    final signed = registration(
      fakeAgent,
      content: '{"name":"Remote","model":"Claude Sonnet"}',
    );
    // Same id and content, but another key's signature over the id.
    final forged = NostrEvent.fromJson({
      ...signed.toJson(),
      'sig': nostr.Event.from(
        kind: 1,
        content: 'other',
        secretKey: fakeAgentKeys.secret,
      ).sig,
    });
    final profiles = [
      signedAgentProfile(
        fakeAgentKeys,
        fakeOwner,
        content: '{"name":"Remote"}',
      ),
    ];
    Future<List<OwnedAgentDirectoryEntry>> load(NostrEvent event) =>
        loadOwnedAgentDirectory(
          fakeOwner.public,
          (filter) async => filter.kinds.contains(30177) ? [event] : profiles,
        );

    expect((await load(signed)).single.model, 'Claude Sonnet');
    final found = (await load(forged)).single;
    expect(found.pubkey, fakeAgent);
    expect(found.model, isNull);
  });

  test('recipient discovery requires the latest signed registration and a '
      'matching owner profile', () async {
    final hosted = registration(fakeAgent, runtime: null);
    final profile = signedAgentProfile(fakeAgentKeys, fakeOwner);
    final forgedNewer = NostrEvent.fromJson({
      ...registration(fakeAgent, runtime: null, createdAt: 2).toJson(),
      'sig': '0' * 128,
    });
    final otherOwner = nostr.Keys('0' * 63 + '3');
    final cases = <(String, List<NostrEvent>, NostrEvent, bool, bool)>[
      ('desktop', [registration(fakeAgent)], profile, true, true),
      ('no runtime', [hosted], profile, false, true),
      for (final runtime in ['goose', 'Block/Goose'])
        (
          runtime,
          [registration(fakeAgent, runtime: runtime)],
          profile,
          false,
          true,
        ),
      for (final content in ['{"name":"  "}', '{}', 'instance', '[]'])
        (
          content,
          [registration(fakeAgent, content: content)],
          profile,
          false,
          false,
        ),
      ('unregistered', [], profile, false, false),
      ('invalid latest', [hosted, forgedNewer], profile, false, false),
      (
        'wrong owner',
        [hosted],
        signedAgentProfile(fakeAgentKeys, otherOwner),
        false,
        false,
      ),
      (
        'forged profile',
        [hosted],
        NostrEvent.fromJson({...profile.toJson(), 'sig': '0' * 128}),
        false,
        false,
      ),
    ];
    for (final (label, registrations, profile, listed, recipient) in cases) {
      final session = _Session(registrations, [profile]);
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(_RelayConfig.new),
          myPubkeyProvider.overrideWithValue(fakeOwner.public),
          relaySessionProvider.overrideWith(() => session),
        ],
      );
      try {
        container.read(relaySessionProvider.notifier);
        session.connect();
        expect(
          await container.read(ownedAgentDirectoryProvider.future),
          hasLength(listed ? 1 : 0),
          reason: label,
        );
        expect(
          (await container.read(
            ownedAgentRecipientsProvider.future,
          )).map((entry) => entry.pubkey),
          recipient ? [fakeAgent] : const [],
          reason: label,
        );
      } finally {
        container.dispose();
      }
    }
  });
}

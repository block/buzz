import 'dart:async';
import '../../helpers/remote_agents_fixture.dart';
import 'package:buzz/features/profile/presence_cache_provider.dart';
import 'package:buzz/shared/agents/owned_agent_directory.dart';
import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _Presence extends PresenceCacheNotifier {
  @override
  Map<String, String> build() => {
    'hosted': 'offline',
    'desktop': 'offline',
    'human': 'away',
  };
}

void main() {
  for (final failure in [false, true]) {
    test(
      'registered hosted agent is Online with account lookup ${failure ? 'failed' : 'empty'}',
      () async {
        final account = FakeRegistry(signedIn: !failure);
        final controller = RelayFakeController(account);
        addTearDown(controller.dispose);
        addTearDown(account.dispose);
        Future<List<OwnedAgentDirectoryEntry>> directory = Future.value([
          const OwnedAgentDirectoryEntry(
            pubkey: 'hosted',
            runtime: 'block.builderlab/builderlab',
          ),
          const OwnedAgentDirectoryEntry(
            pubkey: 'desktop',
            runtime: 'block.claude-code/claude-code',
          ),
        ]);
        final container = ProviderContainer(
          overrides: [
            savedRemoteAvailabilityProvider.overrideWith((ref) async => null),
            remoteAgentControllerProvider.overrideWithValue(controller),
            presenceCacheProvider.overrideWith(_Presence.new),
            ownedAgentDirectoryProvider.overrideWith((ref) => directory),
            availableRemoteAgentsProvider.overrideWith((ref) async {
              if (failure) throw StateError('Account unavailable');
              return {};
            }),
          ],
        );
        addTearDown(container.dispose);
        await container.read(savedRemoteAvailabilityProvider.future);
        await container.read(ownedAgentDirectoryProvider.future);
        try {
          await container.read(availableRemoteAgentsProvider.future);
        } catch (_) {}
        expect(container.read(presentedPresenceProvider), {
          'hosted': 'online',
          'desktop': 'offline',
          'human': 'away',
        });
        expect(container.read(presenceCacheProvider)['hosted'], 'offline');

        // A successful registry snapshot is authoritative even if the other
        // community's signed registration still exists after deletion.
        controller.registryLoaded = true;
        await controller.run(() async {});
        await container.pump();
        expect(container.read(presentedPresenceProvider)['hosted'], 'offline');
        controller.registryLoaded = false;
        controller.deletedAgents = {'hosted'};
        await controller.run(() async {});
        await container.pump();
        expect(container.read(presentedPresenceProvider)['hosted'], 'offline');
        controller.deletedAgents = {};
        await controller.run(() async {});
        await container.pump();
        expect(container.read(presentedPresenceProvider)['hosted'], 'online');

        // A new community must not reuse the previous directory while loading.
        final next = Completer<List<OwnedAgentDirectoryEntry>>();
        directory = next.future;
        container.invalidate(ownedAgentDirectoryProvider);
        await container.pump();
        expect(container.read(presentedPresenceProvider)['hosted'], 'offline');
        next.complete([]);
        await container.read(savedRemoteAvailabilityProvider.future);
        await container.read(ownedAgentDirectoryProvider.future);
        await container.pump();
        expect(container.read(presentedPresenceProvider)['hosted'], 'offline');
      },
    );
  }
}

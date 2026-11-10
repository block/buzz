import 'package:buzz/app.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/invites/invite_join_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  test(
    'invite recovery propagates fresh-read failure despite cached memberships',
    () async {
      final keys = nostr.Keys.generate();
      final scope = InviteJoinRecoveryScope(
        relayHttpOrigin: 'https://relay.example',
        nsec: keys.nsec,
      );
      final channels = _CachedChannels();
      final recovery = Provider(
        (ref) => buildMobileInviteJoinRecovery(ref, scope),
      );
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(() => _Config(keys.nsec)),
          relaySessionProvider.overrideWith(_Connected.new),
          activeCommunityProvider.overrideWith(
            (ref) async => Community.create(
              name: 'Test',
              relayUrl: scope.relayHttpOrigin,
              nsec: keys.nsec,
            ),
          ),
          channelsProvider.overrideWith(() => channels),
        ],
      );
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await expectLater(
        container.read(recovery).ensureStarterChannels(),
        throwsA(
          isA<StateError>().having(
            (e) => e.message,
            'message',
            'membership fetch failed',
          ),
        ),
      );
      expect(channels.freshReads, 1);
      expect(container.read(channelsProvider).requireValue, hasLength(2));
    },
  );
}

class _CachedChannels extends ChannelsNotifier {
  int freshReads = 0;
  @override
  Future<List<Channel>> build() async => [
    for (final name in ['general', 'welcome-everyone'])
      Channel(
        id: name,
        name: name,
        channelType: 'stream',
        visibility: 'open',
        description: '',
        createdBy: 'me',
        createdAt: DateTime.utc(2026),
        memberCount: 1,
        isMember: true,
      ),
  ];
  // A best-effort refresh can retain stale data without failing its caller.
  @override
  Future<void> refresh({bool fetchDirectory = false}) async {}
  @override
  Future<List<Channel>> fetchFresh({bool fetchDirectory = false}) async {
    freshReads++;
    throw StateError('membership fetch failed');
  }
}

class _Config extends RelayConfigNotifier {
  _Config(this.nsec);
  final String nsec;
  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: nsec);
}

class _Connected extends RelaySessionNotifier {
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);
}

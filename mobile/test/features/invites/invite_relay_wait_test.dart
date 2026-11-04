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

const _relay = 'https://relay.example';

void main() {
  late nostr.Keys keys;
  late InviteJoinRecoveryScope scope;
  late _Channels channels;
  late ProviderContainer container;
  late Provider<InviteJoinRecovery> recovery;

  setUp(() {
    keys = nostr.Keys.generate();
    scope = InviteJoinRecoveryScope(relayHttpOrigin: _relay, nsec: keys.nsec);
    channels = _Channels();
    // Production builds a fresh recovery for every attempt.
    recovery = Provider((ref) => buildMobileInviteJoinRecovery(ref, scope));
    container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(() => _Config(keys.nsec)),
        relaySessionProvider.overrideWith(_Session.new),
        activeCommunityProvider.overrideWith(
          (ref) async =>
              Community.create(name: 'Test', relayUrl: _relay, nsec: keys.nsec),
        ),
        channelsProvider.overrideWith(() => channels),
      ],
    );
    addTearDown(container.dispose);
  });

  Future<String?> attempt() {
    container.invalidate(recovery);
    return container.read(recovery).ensureStarterChannels();
  }

  void setStatus(SessionStatus status) =>
      (container.read(relaySessionProvider.notifier) as _Session).set(status);

  _Config config() => container.read(relayConfigProvider.notifier) as _Config;

  test('retry succeeds after an interrupted connection wait', () async {
    await container.read(activeCommunityProvider.future);
    setStatus(SessionStatus.connecting);
    final first = attempt();
    await pumpEventQueue();
    config().update(baseUrl: 'https://other.example', nsec: keys.nsec);
    setStatus(SessionStatus.reconnecting);
    await expectLater(first, throwsA(isA<StateError>()));

    config().update(baseUrl: _relay, nsec: keys.nsec);
    setStatus(SessionStatus.connected);
    expect(await attempt(), 'welcome-everyone');
  });

  test('retry waits for a connection lost after an earlier wait', () async {
    await container.read(activeCommunityProvider.future);
    setStatus(SessionStatus.connected);
    expect(await attempt(), 'welcome-everyone');
    expect(channels.freshReads, 1);

    setStatus(SessionStatus.disconnected);
    final retry = attempt();
    await pumpEventQueue();
    expect(channels.freshReads, 1);

    setStatus(SessionStatus.connected);
    expect(await retry, 'welcome-everyone');
    expect(channels.freshReads, 2);
  });
}

class _Channels extends ChannelsNotifier {
  int freshReads = 0;
  final _starters = [
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
  @override
  Future<List<Channel>> build() async => _starters;
  @override
  Future<List<Channel>> fetchFresh({bool fetchDirectory = false}) async {
    freshReads++;
    return _starters;
  }
}

class _Config extends RelayConfigNotifier {
  _Config(this.nsec);
  final String nsec;
  @override
  RelayConfig build() => RelayConfig(baseUrl: _relay, nsec: nsec);
}

class _Session extends RelaySessionNotifier {
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);
  void set(SessionStatus status) => state = SessionState(status: status);
}

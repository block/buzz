import 'package:buzz/features/channels/read_state/read_state_format.dart';
import 'package:buzz/features/channels/read_state/read_state_provider.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

/// Drives the production [ReadStateNotifier] (real bookkeeping, real
/// [ReadStateManager]) against an inert fake relay session, so a defect in
/// the forced-unread map removal or the manager-emission path fails here
/// instead of being masked by a fake notifier.
void main() {
  const channelId = 'chan-1';
  const otherChannelId = 'chan-2';
  final msgKey = msgContextKey('msg-1');
  final otherMsgKey = msgContextKey('msg-2');

  late ProviderContainer container;

  Future<ReadStateNotifier> pumpNotifier() async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final nsec = nostr.Keys.generate().nsec;

    container = ProviderContainer(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        relayConfigProvider.overrideWith(() => _FakeRelayConfig(nsec)),
        relaySessionProvider.overrideWith(_FakeRelaySession.new),
        activeCommunityProvider.overrideWith((ref) async => null),
        appLifecycleProvider.overrideWith(_FakeAppLifecycle.new),
      ],
    );
    addTearDown(container.dispose);

    final notifier = container.read(readStateProvider.notifier);
    // Let the async activeCommunity resolution, the resulting rebuild, and
    // the manager initialize() microtasks run.
    await container.read(activeCommunityProvider.future);
    for (var i = 0; i < 10 && !container.read(readStateProvider).isReady; i++) {
      await Future<void>.delayed(Duration.zero);
    }
    expect(container.read(readStateProvider).isReady, isTrue);
    return notifier;
  }

  ReadStateState state() => container.read(readStateProvider);

  test('message unread → read → unread round-trips through the real '
      'notifier', () async {
    final notifier = await pumpNotifier();

    // Force the message unread.
    notifier.markContextUnread(msgKey, channelId: channelId);
    expect(state().isForcedUnread(msgKey), isTrue);
    expect(state().locallyForcedChannelIds, {channelId});

    // Mark read: the force flag must drop out of the production map AND the
    // marker must land in the manager's effective contexts.
    notifier.markContextRead(msgKey, 1000);
    expect(state().isForcedUnread(msgKey), isFalse);
    expect(state().forcedUnreadContexts, isEmpty);
    expect(state().effectiveTimestamp(msgKey), 1000);

    // Force unread again: read markers are monotonic, so the flag is the
    // only mechanism — it must win even though the marker persists.
    notifier.markContextUnread(msgKey, channelId: channelId);
    expect(state().isForcedUnread(msgKey), isTrue);
    expect(state().effectiveTimestamp(msgKey), 1000);
  });

  test('explicit channel-level Mark read clears forced message entries in '
      'that channel only', () async {
    final notifier = await pumpNotifier();

    notifier.markContextUnread(msgKey, channelId: channelId);
    notifier.markContextUnread(otherMsgKey, channelId: otherChannelId);

    // Channel tile "Mark as read" passes clearForcedMessages: true.
    notifier.markContextRead(channelId, 2000, clearForcedMessages: true);

    expect(state().isForcedUnread(msgKey), isFalse);
    expect(state().isForcedUnread(otherMsgKey), isTrue);
    expect(state().locallyForcedChannelIds, {otherChannelId});
    expect(state().effectiveTimestamp(channelId), 2000);
  });

  test(
    'automatic channel-open read preserves forced message entries',
    () async {
      final notifier = await pumpNotifier();

      notifier.markContextUnread(msgKey, channelId: channelId);

      // Channel open marks the channel read without clearForcedMessages.
      notifier.markContextRead(channelId, 3000);

      expect(state().isForcedUnread(msgKey), isTrue);
      expect(state().locallyForcedChannelIds, {channelId});
      expect(state().effectiveTimestamp(channelId), 3000);
    },
  );

  test('automatic channel-open read still clears a channel-level force for '
      'the same channel', () async {
    final notifier = await pumpNotifier();

    // Channel forced unread from the tile, message forced from the sheet.
    notifier.markContextUnread(channelId, channelId: channelId);
    notifier.markContextUnread(msgKey, channelId: channelId);

    notifier.markContextRead(channelId, 4000);

    // Opening the channel satisfies the channel-scoped force, but the
    // deliberate message-level force survives until acted on.
    expect(state().isForcedUnread(channelId), isFalse);
    expect(state().isForcedUnread(msgKey), isTrue);
    expect(state().locallyForcedChannelIds, {channelId});
  });
}

class _FakeRelayConfig extends RelayConfigNotifier {
  final String nsec;

  _FakeRelayConfig(this.nsec);

  @override
  RelayConfig build() => RelayConfig(baseUrl: 'http://localhost:1', nsec: nsec);
}

/// Relay session that never connects and returns no history, keeping the
/// manager fully local while exercising its real code paths.
class _FakeRelaySession extends RelaySessionNotifier {
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async => () {};
}

class _FakeAppLifecycle extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}

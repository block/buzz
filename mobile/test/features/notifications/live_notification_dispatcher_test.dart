import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/notifications/live_notification_dispatcher.dart';
import 'package:buzz/shared/notifications/notifications.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  ProviderContainer container({
    required _FakeBridge bridge,
    bool previewsEnabled = false,
    AndroidNotificationPermission permission =
        AndroidNotificationPermission.granted,
    bool priorityChannelEnabled = true,
    bool activityChannelEnabled = true,
  }) {
    final result = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(
          () => _RelayConfigNotifier('https://relay.example'),
        ),
        myPubkeyProvider.overrideWithValue('me'),
        notificationSettingsProvider.overrideWith(
          () => _SettingsNotifier(
            NotificationSettingsState(
              alertsEnabled: true,
              priorityEnabled: true,
              activityEnabled: true,
              previewsEnabled: previewsEnabled,
              permission: permission,
              priorityChannelEnabled: priorityChannelEnabled,
              activityChannelEnabled: activityChannelEnabled,
            ),
          ),
        ),
        androidNotificationBridgeProvider.overrideWithValue(bridge),
      ],
    );
    addTearDown(result.dispose);
    addTearDown(bridge.dispose);
    return result;
  }

  test('redacts notification previews by default', () async {
    final bridge = _FakeBridge();
    final scope = container(bridge: bridge);

    await scope
        .read(liveNotificationDispatcherProvider)
        .dispatch(
          event: _event(content: 'private message'),
          channel: _channel,
          myPubkey: 'me',
          senderName: 'Alice',
        );

    expect(bridge.shows, [
      const _ShowCall(
        title: 'Buzz',
        body: 'New message',
        channel: 'activity',
        route: 'buzz://message?channel=channel-1&id=event-1',
      ),
    ]);
  });

  test('shows sender, channel, and body only after preview opt-in', () async {
    final bridge = _FakeBridge();
    final scope = container(bridge: bridge, previewsEnabled: true);

    await scope
        .read(liveNotificationDispatcherProvider)
        .dispatch(
          event: _event(content: 'private message'),
          channel: _channel,
          myPubkey: 'me',
          senderName: 'Alice',
        );

    expect(bridge.shows.single.title, 'New message in #general');
    expect(bridge.shows.single.body, 'private message');
  });

  test('requires the selected Android category to remain enabled', () async {
    final bridge = _FakeBridge();
    final scope = container(bridge: bridge, activityChannelEnabled: false);

    await scope
        .read(liveNotificationDispatcherProvider)
        .dispatch(event: _event(), channel: _channel, myPubkey: 'me');

    expect(bridge.shows, isEmpty);
  });

  test('does not dispatch after Android permission is revoked', () async {
    final bridge = _FakeBridge();
    final scope = container(
      bridge: bridge,
      permission: AndroidNotificationPermission.denied,
    );

    await scope
        .read(liveNotificationDispatcherProvider)
        .dispatch(event: _event(), channel: _channel, myPubkey: 'me');

    expect(bridge.shows, isEmpty);
  });

  test('deduplicates successful delivery but retries native failure', () async {
    final bridge = _FakeBridge(failuresRemaining: 1);
    final scope = container(bridge: bridge);
    final dispatcher = scope.read(liveNotificationDispatcherProvider);

    await dispatcher.dispatch(
      event: _event(),
      channel: _channel,
      myPubkey: 'me',
    );
    await dispatcher.dispatch(
      event: _event(),
      channel: _channel,
      myPubkey: 'me',
    );
    await dispatcher.dispatch(
      event: _event(),
      channel: _channel,
      myPubkey: 'me',
    );

    expect(bridge.attempts, 2);
    expect(bridge.shows, hasLength(1));
  });

  test('starts a fresh dedupe scope after a community change', () async {
    final bridge = _FakeBridge();
    final scope = container(bridge: bridge);

    await scope
        .read(liveNotificationDispatcherProvider)
        .dispatch(event: _event(), channel: _channel, myPubkey: 'me');
    scope
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://other-relay.example');
    await Future<void>.delayed(Duration.zero);
    await scope
        .read(liveNotificationDispatcherProvider)
        .dispatch(event: _event(), channel: _channel, myPubkey: 'me');

    expect(bridge.shows, hasLength(2));
  });
}

class _FakeBridge extends AndroidNotificationBridge {
  _FakeBridge({this.failuresRemaining = 0});

  int failuresRemaining;
  int attempts = 0;
  final List<_ShowCall> shows = [];

  @override
  Future<void> show({
    required int id,
    required String channel,
    required String title,
    required String body,
    required String route,
  }) async {
    attempts++;
    if (failuresRemaining > 0) {
      failuresRemaining--;
      throw PlatformException(code: 'test_failure');
    }
    shows.add(
      _ShowCall(title: title, body: body, channel: channel, route: route),
    );
  }
}

class _ShowCall {
  const _ShowCall({
    required this.title,
    required this.body,
    required this.channel,
    required this.route,
  });

  final String title;
  final String body;
  final String channel;
  final String route;

  @override
  bool operator ==(Object other) =>
      other is _ShowCall &&
      other.title == title &&
      other.body == body &&
      other.channel == channel &&
      other.route == route;

  @override
  int get hashCode => Object.hash(title, body, channel, route);
}

class _SettingsNotifier extends NotificationSettingsNotifier {
  _SettingsNotifier(this.initialState);

  final NotificationSettingsState initialState;

  @override
  NotificationSettingsState build() => initialState;
}

class _RelayConfigNotifier extends RelayConfigNotifier {
  _RelayConfigNotifier(this.url);

  final String url;

  @override
  RelayConfig build() => RelayConfig(baseUrl: url);
}

NostrEvent _event({String content = 'hello'}) => NostrEvent(
  id: 'event-1',
  pubkey: 'alice',
  createdAt: 100,
  kind: EventKind.streamMessageV2,
  tags: const [
    ['h', 'channel-1'],
  ],
  content: content,
  sig: 'sig',
);

final _channel = Channel(
  id: 'channel-1',
  name: 'general',
  channelType: 'stream',
  visibility: 'open',
  description: '',
  createdBy: 'creator',
  createdAt: DateTime.fromMillisecondsSinceEpoch(0, isUtc: true),
  memberCount: 2,
  isMember: true,
);

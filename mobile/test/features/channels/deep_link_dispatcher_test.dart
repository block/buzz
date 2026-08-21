import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/deep_link_dispatcher.dart';
import 'package:buzz/features/invites/invite_join_provider.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/community/community_storage_test.dart';

void main() {
  testWidgets('queues deep links in arrival order until acknowledged', (
    tester,
  ) async {
    final controller = StreamController<Uri>();
    PendingDeepLinkNotifier.debugUriStreamOverride = controller.stream;
    addTearDown(() async {
      PendingDeepLinkNotifier.debugUriStreamOverride = null;
      await controller.close();
    });
    final container = ProviderContainer();
    addTearDown(container.dispose);
    container.read(pendingDeepLinkProvider);

    const channelId = '580ca78b-9dae-46f3-8854-bd671853ba32';
    final firstId = 'aa' * 32;
    final secondId = 'bb' * 32;
    controller
      ..add(Uri.parse('buzz://message?channel=$channelId&id=$firstId'))
      ..add(Uri.parse('buzz://message?channel=$channelId&id=$secondId'));
    await tester.pump();

    expect(
      container.read(pendingDeepLinkProvider),
      MessageDeepLink(channelId: channelId, messageId: firstId),
    );
    container.read(pendingDeepLinkProvider.notifier).consume();
    expect(
      container.read(pendingDeepLinkProvider),
      MessageDeepLink(channelId: channelId, messageId: secondId),
    );
    container.read(pendingDeepLinkProvider.notifier).consume();
    expect(container.read(pendingDeepLinkProvider), isNull);
  });

  testWidgets('drops a missing channel and dispatches the next queued link', (
    tester,
  ) async {
    const missing = ChannelDeepLink(channelId: 'missing-channel');
    const next = ChannelDeepLink(channelId: 'channel-1');
    final pending = _QueuedPendingDeepLinkNotifier([missing, next]);

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          pendingDeepLinkProvider.overrideWith(() => pending),
          channelsProvider.overrideWith(
            () => _FakeChannelsNotifier(Future.value([_channel])),
          ),
        ],
        child: MaterialApp(
          home: DeepLinkDispatcher(
            destinationBuilder: (channel, link) =>
                _CapturedDestination(channel: channel, link: link),
            child: const Scaffold(body: SizedBox()),
          ),
        ),
      ),
    );

    await tester.pumpAndSettle();

    expect(pending.consumeCalls, 2);
    final destination = tester.widget<_CapturedDestination>(
      find.byType(_CapturedDestination),
    );
    expect(destination.channel.id, 'channel-1');
    expect(destination.link, same(next));
  });

  testWidgets('dispatches a link that is already ready on mount', (
    tester,
  ) async {
    const link = MessageDeepLink(
      channelId: 'channel-1',
      messageId: 'message-2',
      threadRootId: 'message-1',
    );

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          pendingDeepLinkProvider.overrideWith(
            () => _FakePendingDeepLinkNotifier(link),
          ),
          channelsProvider.overrideWith(
            () => _FakeChannelsNotifier(Future.value([_channel])),
          ),
        ],
        child: MaterialApp(
          home: DeepLinkDispatcher(
            destinationBuilder: (channel, link) =>
                _CapturedDestination(channel: channel, link: link),
            child: const Scaffold(body: SizedBox()),
          ),
        ),
      ),
    );

    await tester.pumpAndSettle();

    final destination = tester.widget<_CapturedDestination>(
      find.byType(_CapturedDestination),
    );
    final messageLink = destination.link as MessageDeepLink;
    expect(destination.channel.id, 'channel-1');
    expect(messageLink.messageId, 'message-2');
    expect(messageLink.threadRootId, 'message-1');
  });

  testWidgets('dispatches a channel-only link to the channel root', (
    tester,
  ) async {
    const link = ChannelDeepLink(channelId: 'channel-1');

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          pendingDeepLinkProvider.overrideWith(
            () => _FakePendingDeepLinkNotifier(link),
          ),
          channelsProvider.overrideWith(
            () => _FakeChannelsNotifier(Future.value([_channel])),
          ),
        ],
        child: MaterialApp(
          home: DeepLinkDispatcher(
            destinationBuilder: (channel, link) =>
                _CapturedDestination(channel: channel, link: link),
            child: const Scaffold(body: SizedBox()),
          ),
        ),
      ),
    );

    await tester.pumpAndSettle();

    final destination = tester.widget<_CapturedDestination>(
      find.byType(_CapturedDestination),
    );
    expect(destination.channel.id, 'channel-1');
    expect(destination.link, same(link));
  });

  testWidgets('retains invite and surfaces prepare failure', (tester) async {
    const link = InviteDeepLink(
      relayUrl: 'wss://relay.example.com',
      code: 'invite-code',
    );
    final container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(_ThrowingCommunityStorage()),
        pendingDeepLinkProvider.overrideWith(
          () => _FakePendingDeepLinkNotifier(link),
        ),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: const MaterialApp(
          home: DeepLinkDispatcher(child: Scaffold(body: SizedBox())),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(container.read(pendingDeepLinkProvider), same(link));
    expect(container.read(inviteJoinProvider).status, InviteJoinStatus.idle);
    expect(
      find.text(
        'Could not open this invite. Re-open the invite link to try again.',
      ),
      findsOneWidget,
    );
    expect(tester.takeException(), isNull);
  });

  testWidgets('prepares an invite once while listeners re-enter', (
    tester,
  ) async {
    const link = InviteDeepLink(
      relayUrl: 'wss://relay.example.com',
      code: 'invite-code',
    );
    final storage = _CountingCommunityStorage();
    final pending = _RecordingPendingDeepLinkNotifier(link);
    final container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(storage),
        pendingDeepLinkProvider.overrideWith(() => pending),
        channelsProvider.overrideWith(
          () => _FakeChannelsNotifier(Future.value([_channel])),
        ),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: const MaterialApp(
          home: DeepLinkDispatcher(child: Scaffold(body: SizedBox())),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(storage.loadCalls, 1);
    expect(pending.consumeCalls, 1);
    expect(find.text('Join this Buzz community?'), findsOneWidget);
  });

  testWidgets('waits for an invite modal before preparing the next invite', (
    tester,
  ) async {
    const first = InviteDeepLink(
      relayUrl: 'wss://relay.example.com',
      code: 'invite-one',
    );
    const second = InviteDeepLink(
      relayUrl: 'wss://relay.example.com',
      code: 'invite-two',
    );
    final pending = _QueuedPendingDeepLinkNotifier([first, second]);
    final container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(
          CommunityStorage(secure: FakeSecureStorage()),
        ),
        pendingDeepLinkProvider.overrideWith(() => pending),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: const MaterialApp(
          home: DeepLinkDispatcher(child: Scaffold(body: SizedBox())),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(pending.consumeCalls, 1);
    expect(pending.current, same(second));
    expect(container.read(inviteJoinProvider).invite, same(first));
    expect(find.text('Join this Buzz community?'), findsOneWidget);

    await tester.tap(find.widgetWithText(OutlinedButton, 'Cancel'));
    await tester.pumpAndSettle();

    expect(pending.consumeCalls, 2);
    expect(pending.current, isNull);
    expect(container.read(inviteJoinProvider).invite, same(second));
    expect(find.text('Join this Buzz community?'), findsOneWidget);
  });

  testWidgets('dispatches a queued channel after preparing an invite', (
    tester,
  ) async {
    const invite = InviteDeepLink(
      relayUrl: 'wss://relay.example.com',
      code: 'invite-code',
    );
    const channelLink = ChannelDeepLink(channelId: 'channel-1');
    final pending = _QueuedPendingDeepLinkNotifier([invite, channelLink]);

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          communityStorageProvider.overrideWithValue(
            CommunityStorage(secure: FakeSecureStorage()),
          ),
          pendingDeepLinkProvider.overrideWith(() => pending),
          channelsProvider.overrideWith(
            () => _FakeChannelsNotifier(Future.value([_channel])),
          ),
        ],
        child: MaterialApp(
          home: DeepLinkDispatcher(
            destinationBuilder: (channel, link) =>
                _CapturedDestination(channel: channel, link: link),
            child: const Scaffold(body: SizedBox()),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(pending.consumeCalls, 1);
    expect(pending.current, same(channelLink));
    expect(find.text('Join this Buzz community?'), findsOneWidget);
    expect(find.byType(_CapturedDestination), findsNothing);

    await tester.tap(find.widgetWithText(OutlinedButton, 'Cancel'));
    await tester.pumpAndSettle();

    expect(pending.consumeCalls, 2);
    expect(pending.current, isNull);
    final destination = tester.widget<_CapturedDestination>(
      find.byType(_CapturedDestination),
    );
    expect(destination.link, same(channelLink));
  });

  testWidgets(
    'dispatches invite before auth while leaving message links parked',
    (tester) async {
      final inviteStorage = CommunityStorage(secure: FakeSecureStorage());
      final inviteContainer = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(inviteStorage),
          pendingDeepLinkProvider.overrideWith(
            () => _FakePendingDeepLinkNotifier(
              const InviteDeepLink(
                relayUrl: 'wss://relay.example.com',
                code: 'invite-code',
              ),
            ),
          ),
        ],
      );
      addTearDown(inviteContainer.dispose);

      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: inviteContainer,
          child: const MaterialApp(
            home: DeepLinkDispatcher(
              dispatchMessageLinks: false,
              child: Scaffold(body: Text('Pairing')),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Join this Buzz community?'), findsOneWidget);
      expect(inviteContainer.read(pendingDeepLinkProvider), isNull);

      final messageContainer = ProviderContainer(
        overrides: [
          pendingDeepLinkProvider.overrideWith(
            () => _FakePendingDeepLinkNotifier(
              const MessageDeepLink(
                channelId: 'channel-1',
                messageId: 'message-1',
              ),
            ),
          ),
        ],
      );
      addTearDown(messageContainer.dispose);
      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: messageContainer,
          child: const MaterialApp(
            home: DeepLinkDispatcher(
              dispatchMessageLinks: false,
              child: Scaffold(body: Text('Pairing')),
            ),
          ),
        ),
      );
      await tester.pump();

      expect(
        messageContainer.read(pendingDeepLinkProvider),
        isA<MessageDeepLink>(),
      );
      expect(find.text('Pairing'), findsOneWidget);
    },
  );
}

final _channel = Channel(
  id: 'channel-1',
  name: 'general',
  channelType: 'stream',
  visibility: 'open',
  description: 'General discussion',
  createdBy: 'creator',
  createdAt: DateTime(2026),
  memberCount: 2,
  isMember: true,
);

class _CountingCommunityStorage extends CommunityStorage {
  int loadCalls = 0;

  @override
  Future<List<Community>> loadAll() async {
    loadCalls++;
    return [];
  }
}

class _ThrowingCommunityStorage extends CommunityStorage {
  @override
  Future<List<Community>> loadAll() async {
    throw StateError('secure storage unavailable');
  }
}

class _QueuedPendingDeepLinkNotifier extends PendingDeepLinkNotifier {
  _QueuedPendingDeepLinkNotifier(List<BuzzDeepLink> links)
    : _links = List.of(links);

  final List<BuzzDeepLink> _links;
  int consumeCalls = 0;

  BuzzDeepLink? get _firstOrNull => _links.isEmpty ? null : _links.first;
  BuzzDeepLink? get current => _firstOrNull;

  @override
  BuzzDeepLink? build() => _firstOrNull;

  @override
  void consume() {
    consumeCalls++;
    _links.removeAt(0);
    state = _firstOrNull;
  }
}

class _RecordingPendingDeepLinkNotifier extends PendingDeepLinkNotifier {
  _RecordingPendingDeepLinkNotifier(this.link);

  final BuzzDeepLink link;
  int consumeCalls = 0;

  @override
  BuzzDeepLink? build() => link;

  @override
  void consume() {
    consumeCalls++;
    super.consume();
  }
}

class _FakePendingDeepLinkNotifier extends PendingDeepLinkNotifier {
  _FakePendingDeepLinkNotifier(this.link);

  final BuzzDeepLink link;

  @override
  BuzzDeepLink? build() => link;
}

class _FakeChannelsNotifier extends ChannelsNotifier {
  _FakeChannelsNotifier(this.channels);

  final Future<List<Channel>> channels;

  @override
  Future<List<Channel>> build() => channels;
}

class _CapturedDestination extends StatelessWidget {
  const _CapturedDestination({required this.channel, required this.link});

  final Channel channel;
  final BuzzDeepLink link;

  @override
  Widget build(BuildContext context) => const SizedBox();
}

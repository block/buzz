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
    expect(destination.channel.id, 'channel-1');
    expect(destination.link.messageId, 'message-2');
    expect(destination.link.threadRootId, 'message-1');
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
  final MessageDeepLink link;

  @override
  Widget build(BuildContext context) => const SizedBox();
}

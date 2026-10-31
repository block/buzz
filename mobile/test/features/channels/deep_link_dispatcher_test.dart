import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/deep_link_dispatcher.dart';
import 'package:buzz/features/invites/invite_join_provider.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/community/community_storage_test.dart';

void main() {
  for (final outcome in ['handled', 'declined', 'failed']) {
    testWidgets('hosted handler drains queued links after $outcome setup', (
      tester,
    ) async {
      final BuzzDeepLink first = outcome == 'declined'
          ? const AddCommunityDeepLink(
              relayUrl: 'wss://one.communities.buzz.xyz',
              name: 'One',
            )
          : const InviteDeepLink(
              relayUrl: 'wss://one.communities.buzz.xyz',
              code: 'same',
            );
      final duplicate = outcome == 'declined'
          ? first
          : InviteDeepLink(
              relayUrl: 'wss://one.communities.buzz.xyz',
              code: 'same',
            );
      const second = InviteDeepLink(
        relayUrl: 'wss://two.communities.buzz.xyz',
        code: 'invite',
      );
      final pending = _QueuedPendingDeepLinkNotifier([
        first,
        duplicate,
        second,
      ]);
      final handled = <BuzzDeepLink>[];
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            pendingDeepLinkProvider.overrideWith(() => pending),
            inviteKeyGeneratorProvider.overrideWithValue(
              () => throw StateError('must not generate'),
            ),
          ],
          child: MaterialApp(
            home: DeepLinkDispatcher(
              dispatchMessageLinks: false,
              onboardingHandler: (_, link) async {
                handled.add(link);
                if (link == first) {
                  if (outcome == 'declined') return false;
                  if (outcome == 'failed') throw StateError('setup failed');
                }
                return true;
              },
              child: const Scaffold(body: SizedBox()),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(handled, [first, duplicate, second]);
      expect(pending.consumeCalls, 3);
    });
  }
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
    final session = _MetadataRelaySession(const []);

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          pendingDeepLinkProvider.overrideWith(() => pending),
          relaySessionProvider.overrideWith(() => session),
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

  for (final visibility in ['open', 'private']) {
    testWidgets('resolves an unlisted $visibility channel from the relay', (
      tester,
    ) async {
      const link = MessageDeepLink(
        channelId: 'unjoined-channel',
        messageId: 'message-2',
        threadRootId: 'message-1',
      );
      final pending = _RecordingPendingDeepLinkNotifier(link);
      final session = _MetadataRelaySession([
        _channelMeta('unjoined-channel', 'agents', visibility, createdAt: 1),
        _channelMeta('unjoined-channel', 'agents-renamed', visibility),
      ]);

      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            pendingDeepLinkProvider.overrideWith(() => pending),
            relaySessionProvider.overrideWith(() => session),
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

      expect(session.filters, hasLength(1));
      expect(session.filters.single.kinds, [39000]);
      expect(session.filters.single.tags['#d'], ['unjoined-channel']);
      expect(pending.consumeCalls, 1);
      if (visibility == 'private') {
        expect(find.byType(_CapturedDestination), findsNothing);
        expect(
          find.text('Channel not found in this workspace'),
          findsOneWidget,
        );
        return;
      }
      final destination = tester.widget<_CapturedDestination>(
        find.byType(_CapturedDestination),
      );
      expect(destination.channel.id, 'unjoined-channel');
      expect(destination.channel.name, 'agents-renamed');
      expect(destination.channel.isMember, isFalse);
      expect(destination.link, same(link));
    });
  }

  testWidgets('holds a link whose lookup ends while dispatch is off', (
    tester,
  ) async {
    const link = ChannelDeepLink(channelId: 'unjoined-channel');
    final pending = _RecordingPendingDeepLinkNotifier(link);
    final gate = Completer<void>();
    final session = _MetadataRelaySession([
      _channelMeta('unjoined-channel', 'agents', 'open'),
    ], gate: gate);
    final dispatch = ValueNotifier(true);
    addTearDown(dispatch.dispose);

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          pendingDeepLinkProvider.overrideWith(() => pending),
          relaySessionProvider.overrideWith(() => session),
          channelsProvider.overrideWith(
            () => _FakeChannelsNotifier(Future.value([_channel])),
          ),
        ],
        child: MaterialApp(
          home: ValueListenableBuilder<bool>(
            valueListenable: dispatch,
            builder: (_, enabled, _) => DeepLinkDispatcher(
              dispatchMessageLinks: enabled,
              destinationBuilder: (channel, link) =>
                  _CapturedDestination(channel: channel, link: link),
              child: const Scaffold(body: SizedBox()),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump();
    expect(session.filters, hasLength(1));

    // Hosted setup appears while the lookup is still waiting on the relay.
    dispatch.value = false;
    await tester.pump();
    gate.complete();
    await tester.pumpAndSettle();

    expect(find.byType(_CapturedDestination), findsNothing);
    expect(pending.consumeCalls, 0);
    expect(find.text('Channel not found in this workspace'), findsNothing);

    // Setup closes: the parked link opens without a new link or channel event.
    dispatch.value = true;
    await tester.pumpAndSettle();

    final destination = tester.widget<_CapturedDestination>(
      find.byType(_CapturedDestination),
    );
    expect(destination.channel.id, 'unjoined-channel');
    expect(destination.link, same(link));
    expect(pending.consumeCalls, 1);
  });

  testWidgets('parks an unlisted channel link until the session connects', (
    tester,
  ) async {
    const link = ChannelDeepLink(channelId: 'unjoined-channel');
    final pending = _RecordingPendingDeepLinkNotifier(link);
    final session = _MetadataRelaySession([
      _channelMeta('unjoined-channel', 'agents', 'open'),
    ], initialStatus: SessionStatus.reconnecting);

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          pendingDeepLinkProvider.overrideWith(() => pending),
          relaySessionProvider.overrideWith(() => session),
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

    // Cached channels are loaded, but no lookup goes to an unauthenticated
    // socket and the link stays parked.
    expect(session.filters, isEmpty);
    expect(pending.consumeCalls, 0);
    expect(find.byType(_CapturedDestination), findsNothing);
    expect(find.byType(SnackBar), findsNothing);

    session.setStatus(SessionStatus.connected);
    await tester.pumpAndSettle();

    expect(session.filters, hasLength(1));
    final destination = tester.widget<_CapturedDestination>(
      find.byType(_CapturedDestination),
    );
    expect(destination.channel.id, 'unjoined-channel');
    expect(destination.link, same(link));
    expect(pending.consumeCalls, 1);
  });

  testWidgets('opens a listed channel link while the session reconnects', (
    tester,
  ) async {
    const link = ChannelDeepLink(channelId: 'channel-1');
    final pending = _RecordingPendingDeepLinkNotifier(link);
    final session = _MetadataRelaySession(
      const [],
      initialStatus: SessionStatus.reconnecting,
    );

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          pendingDeepLinkProvider.overrideWith(() => pending),
          relaySessionProvider.overrideWith(() => session),
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

    expect(session.filters, isEmpty);
    final destination = tester.widget<_CapturedDestination>(
      find.byType(_CapturedDestination),
    );
    expect(destination.channel.id, 'channel-1');
    expect(pending.consumeCalls, 1);
  });

  for (final backBeforeLookupEnds in [false, true]) {
    testWidgets('retries a lookup cancelled by a session drop '
        '(back before it ends: $backBeforeLookupEnds)', (tester) async {
      const link = ChannelDeepLink(channelId: 'unjoined-channel');
      final pending = _RecordingPendingDeepLinkNotifier(link);
      final gate = Completer<void>();
      final session = _MetadataRelaySession([
        _channelMeta('unjoined-channel', 'agents', 'open'),
      ], gate: gate);

      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            pendingDeepLinkProvider.overrideWith(() => pending),
            relaySessionProvider.overrideWith(() => session),
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
      await tester.pump();
      await tester.pump();
      expect(session.filters, hasLength(1));

      session.drop();
      if (backBeforeLookupEnds) {
        session.setStatus(SessionStatus.connected);
      } else {
        await tester.pumpAndSettle();
        expect(pending.consumeCalls, 0);
        expect(find.byType(_CapturedDestination), findsNothing);
        expect(find.byType(SnackBar), findsNothing);
        session.setStatus(SessionStatus.connected);
      }
      await tester.pump();
      gate.complete();
      await tester.pumpAndSettle();

      expect(session.filters, hasLength(2));
      expect(find.byType(SnackBar), findsNothing);
      final destination = tester.widget<_CapturedDestination>(
        find.byType(_CapturedDestination),
      );
      expect(destination.channel.id, 'unjoined-channel');
      expect(destination.link, same(link));
      expect(pending.consumeCalls, 1);
    });
  }

  testWidgets('dispatches a queued link after a slow lookup finishes', (
    tester,
  ) async {
    const slow = ChannelDeepLink(channelId: 'unjoined-channel');
    const next = ChannelDeepLink(channelId: 'channel-1');
    final pending = _QueuedPendingDeepLinkNotifier([slow, next]);
    final gate = Completer<void>();
    final session = _MetadataRelaySession([
      _channelMeta('unjoined-channel', 'agents', 'open'),
    ], gate: gate);
    final opened = <BuzzDeepLink>[];

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          pendingDeepLinkProvider.overrideWith(() => pending),
          relaySessionProvider.overrideWith(() => session),
          channelsProvider.overrideWith(
            () => _FakeChannelsNotifier(Future.value([_channel])),
          ),
        ],
        child: MaterialApp(
          home: DeepLinkDispatcher(
            destinationBuilder: (channel, link) {
              opened.add(link);
              return _CapturedDestination(channel: channel, link: link);
            },
            child: const Scaffold(body: SizedBox()),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump();
    expect(opened, isEmpty);
    expect(pending.consumeCalls, 0);

    gate.complete();
    await tester.pumpAndSettle();

    expect(opened, [slow, next]);
    expect(pending.consumeCalls, 2);
  });

  testWidgets('drops a lookup that ends after the dispatcher unmounts', (
    tester,
  ) async {
    const link = ChannelDeepLink(channelId: 'unjoined-channel');
    final pending = _RecordingPendingDeepLinkNotifier(link);
    final gate = Completer<void>();
    final session = _MetadataRelaySession([
      _channelMeta('unjoined-channel', 'agents', 'open'),
    ], gate: gate);
    final container = ProviderContainer(
      overrides: [
        pendingDeepLinkProvider.overrideWith(() => pending),
        relaySessionProvider.overrideWith(() => session),
        channelsProvider.overrideWith(
          () => _FakeChannelsNotifier(Future.value([_channel])),
        ),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: MaterialApp(
          home: DeepLinkDispatcher(
            destinationBuilder: (channel, link) =>
                _CapturedDestination(channel: channel, link: link),
            child: const Scaffold(body: SizedBox()),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump();
    expect(session.filters, hasLength(1));

    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: const MaterialApp(home: Scaffold(body: Text('Signed out'))),
      ),
    );
    gate.complete();
    await tester.pumpAndSettle();

    expect(find.byType(_CapturedDestination), findsNothing);
    expect(pending.consumeCalls, 0);
    expect(container.read(pendingDeepLinkProvider), same(link));
  });

  testWidgets('reports a failed relay lookup and drops the link', (
    tester,
  ) async {
    const link = ChannelDeepLink(channelId: 'unjoined-channel');
    final pending = _RecordingPendingDeepLinkNotifier(link);
    final session = _MetadataRelaySession(const [], fail: true);

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          pendingDeepLinkProvider.overrideWith(() => pending),
          relaySessionProvider.overrideWith(() => session),
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
    expect(find.byType(_CapturedDestination), findsNothing);
    expect(find.text('Could not open the channel'), findsOneWidget);
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

  testWidgets('switches to the notification community before dispatch', (
    tester,
  ) async {
    final storage = CommunityStorage(secure: FakeSecureStorage());
    await storage.save(_firstCommunity);
    await storage.save(_notificationCommunity);
    await storage.saveActiveId(_firstCommunity.id);
    const link = MessageDeepLink(
      communityId: 'community-2',
      channelId: 'channel-1',
      messageId: 'message-2',
    );

    final container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(storage),
        communitySnapshotWriterProvider.overrideWithValue((_) async {}),
        pendingDeepLinkProvider.overrideWith(
          () => _FakePendingDeepLinkNotifier(link),
        ),
        channelsProvider.overrideWith(
          () => _FakeChannelsNotifier(Future.value([_channel])),
        ),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: MaterialApp(
          home: DeepLinkDispatcher(
            key: const ValueKey('before-community-switch'),
            destinationBuilder: (channel, link) =>
                _CapturedDestination(channel: channel, link: link),
            child: const Scaffold(body: SizedBox()),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(await storage.loadActiveId(), _notificationCommunity.id);
    expect(container.read(pendingDeepLinkProvider), link);

    // Production remounts the community-scoped app subtree after a switch.
    // The parked link is consumed by the replacement dispatcher.
    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
        child: MaterialApp(
          home: DeepLinkDispatcher(
            key: const ValueKey('after-community-switch'),
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
    expect(destination.link, link);
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

  testWidgets('opens retry setup after durable starter recovery fails', (
    tester,
  ) async {
    const link = InviteDeepLink(
      relayUrl: 'wss://relay.example.com',
      code: 'invite-code',
    );
    final container = ProviderContainer(
      overrides: [
        pendingDeepLinkProvider.overrideWith(
          () => _FakePendingDeepLinkNotifier(link),
        ),
        inviteJoinProvider.overrideWith(
          _FailedStarterRecoveryInviteJoinNotifier.new,
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

    expect(find.text('Finish setting up'), findsOneWidget);
    expect(find.text('Retry setup'), findsOneWidget);
    expect(find.text('Starter setup failed'), findsOneWidget);
  });

  testWidgets(
    'shows saved starter recovery progress, then opens welcome-everyone',
    (tester) async {
      const relayUrl = 'wss://relay.example.com';
      const welcomeId = 'welcome-everyone-id';
      final storage = CommunityStorage(secure: FakeSecureStorage());
      await storage.save(
        Community(
          id: 'incomplete-community',
          name: 'Relay',
          relayUrl: relayUrl,
          pubkey: 'pubkey',
          nsec: nostr.Keys.generate().nsec,
          addedAt: DateTime.utc(2026),
          starterSetupIncomplete: true,
        ),
      );
      final recovery = _DeferredInviteJoinRecovery();
      final container = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          pendingDeepLinkProvider.overrideWith(
            () => _FakePendingDeepLinkNotifier(
              const InviteDeepLink(relayUrl: relayUrl, code: 'invite-code'),
            ),
          ),
          inviteJoinRecoveryProvider.overrideWithValue((_) => recovery),
          channelsProvider.overrideWith(
            () =>
                _FakeChannelsNotifier(Future.value([_welcomeEveryoneChannel])),
          ),
        ],
      );
      addTearDown(container.dispose);

      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: container,
          child: MaterialApp(
            home: DeepLinkDispatcher(
              destinationBuilder: (channel, link) =>
                  _CapturedDestination(channel: channel, link: link),
              child: const Scaffold(body: SizedBox()),
            ),
          ),
        ),
      );
      await tester.pump();
      await tester.pump();
      expect(recovery.started.isCompleted, isTrue);

      expect(find.text('Finish setting up'), findsOneWidget);
      expect(find.text('Finishing setup…'), findsOneWidget);
      expect(
        tester
            .widget<FilledButton>(
              find.widgetWithText(FilledButton, 'Finishing setup…'),
            )
            .onPressed,
        isNull,
      );

      recovery.complete(welcomeId);
      await tester.pumpAndSettle();

      expect(find.text('Continue to #welcome-everyone'), findsOneWidget);
      await tester.tap(find.text('Continue to #welcome-everyone'));
      await tester.pumpAndSettle();
      final destination = tester.widget<_CapturedDestination>(
        find.byType(_CapturedDestination),
      );
      expect(destination.link, const ChannelDeepLink(channelId: welcomeId));
    },
  );

  testWidgets('renders setup-specific recovery failures after membership', (
    tester,
  ) async {
    const relayUrl = 'wss://relay.example.com';
    final storage = CommunityStorage(secure: FakeSecureStorage());
    await storage.save(
      Community(
        id: 'incomplete-community',
        name: 'Relay',
        relayUrl: relayUrl,
        pubkey: 'pubkey',
        nsec: nostr.Keys.generate().nsec,
        addedAt: DateTime.utc(2026),
        starterSetupIncomplete: true,
      ),
    );
    final recovery = _DeferredInviteJoinRecovery();
    final container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(storage),
        pendingDeepLinkProvider.overrideWith(
          () => _FakePendingDeepLinkNotifier(
            const InviteDeepLink(relayUrl: relayUrl, code: 'invite-code'),
          ),
        ),
        inviteJoinRecoveryProvider.overrideWithValue((_) => recovery),
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
    await tester.pump();
    await tester.pump();
    expect(recovery.started.isCompleted, isTrue);

    recovery.completeError(Exception('relay did not respond'));
    await tester.pumpAndSettle();

    expect(
      find.text(
        'Starter channels could not be set up. Retry setup to try again.',
      ),
      findsOneWidget,
    );
    expect(find.text('Retry setup'), findsOneWidget);
    expect(find.textContaining('Could not join this community'), findsNothing);
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

  testWidgets('continues a successful invite into welcome-everyone', (
    tester,
  ) async {
    const invite = InviteDeepLink(
      relayUrl: 'wss://relay.example.com',
      code: 'invite-code',
    );
    final container = ProviderContainer(
      overrides: [
        pendingDeepLinkProvider.overrideWith(
          () => _FakePendingDeepLinkNotifier(invite),
        ),
        channelsProvider.overrideWith(
          () => _FakeChannelsNotifier(Future.value([_welcomeEveryoneChannel])),
        ),
        inviteJoinProvider.overrideWith(_SuccessfulInviteJoinNotifier.new),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: container,
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

    await tester.tap(find.widgetWithText(FilledButton, 'Join'));
    await tester.pumpAndSettle();
    expect(find.text('Continue to #welcome-everyone'), findsOneWidget);

    await tester.tap(find.text('Continue to #welcome-everyone'));
    await tester.pumpAndSettle();

    final destination = tester.widget<_CapturedDestination>(
      find.byType(_CapturedDestination),
    );
    expect(destination.channel.id, 'welcome-everyone-id');
    expect(
      destination.link,
      const ChannelDeepLink(channelId: 'welcome-everyone-id'),
    );
  });
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

final _firstCommunity = Community(
  id: 'community-1',
  name: 'First',
  relayUrl: 'wss://first.example',
  addedAt: DateTime(2026),
);

final _notificationCommunity = Community(
  id: 'community-2',
  name: 'Notification',
  relayUrl: 'wss://notification.example',
  addedAt: DateTime(2026),
);
final _welcomeEveryoneChannel = Channel(
  id: 'welcome-everyone-id',
  name: 'welcome-everyone',
  channelType: 'stream',
  visibility: 'open',
  description: 'Say hi',
  createdBy: 'creator',
  createdAt: DateTime(2026),
  memberCount: 1,
  isMember: true,
);

class _SuccessfulInviteJoinNotifier extends InviteJoinNotifier {
  @override
  InviteJoinState build() => const InviteJoinState();

  @override
  Future<void> prepare(InviteDeepLink invite) async {
    state = InviteJoinState(
      status: InviteJoinStatus.confirming,
      invite: invite,
      host: 'relay.example.com',
      communityName: 'Example',
    );
  }

  @override
  Future<void> confirmJoin() async {
    state = state.copyWith(
      status: InviteJoinStatus.success,
      focusChannelId: 'welcome-everyone-id',
    );
  }
}

class _FailedStarterRecoveryInviteJoinNotifier extends InviteJoinNotifier {
  @override
  InviteJoinState build() => const InviteJoinState();

  @override
  Future<void> prepare(InviteDeepLink invite) async {
    state = InviteJoinState(
      status: InviteJoinStatus.error,
      invite: invite,
      host: 'relay.example.com',
      communityName: 'Relay',
      errorMessage: 'Starter setup failed',
      isStarterSetupRecovery: true,
    );
  }
}

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

NostrEvent _channelMeta(
  String id,
  String name,
  String visibility, {
  int createdAt = 2,
}) => NostrEvent(
  id: 'meta-$id-$createdAt',
  pubkey: 'relay',
  createdAt: createdAt,
  kind: 39000,
  tags: [
    ['d', id],
    ['name', name],
    ['t', 'stream'],
    [visibility == 'private' ? 'private' : 'public'],
  ],
  content: '',
  sig: 'sig',
);

/// Answers kind:39000 lookups with canned metadata and records each filter.
class _MetadataRelaySession extends RelaySessionNotifier {
  _MetadataRelaySession(
    this.metas, {
    this.fail = false,
    this.gate,
    this.initialStatus = SessionStatus.connected,
  });

  final List<NostrEvent> metas;
  final bool fail;

  /// When set, each lookup waits for it, so tests can act mid-lookup.
  final Completer<void>? gate;
  final SessionStatus initialStatus;
  final filters = <NostrFilter>[];
  final _running = <Completer<List<NostrEvent>>>[];

  @override
  SessionState build() => SessionState(status: initialStatus);

  void setStatus(SessionStatus status) => state = SessionState(status: status);

  /// A socket drop cancels every running lookup, then starts reconnecting.
  void drop() {
    for (final lookup in _running) {
      if (!lookup.isCompleted) lookup.completeError(Exception('Lost'));
    }
    _running.clear();
    setStatus(SessionStatus.reconnecting);
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    filters.add(filter);
    final lookup = Completer<List<NostrEvent>>();
    _running.add(lookup);
    Future<void> answer() async {
      if (gate case final gate?) await gate.future;
      if (lookup.isCompleted) return;
      if (fail) {
        lookup.completeError(StateError('relay unavailable'));
        return;
      }
      final ids = filter.tags['#d'] ?? const <String>[];
      lookup.complete([
        for (final event in metas)
          if (ids.contains(event.getTagValue('d'))) event,
      ]);
    }

    unawaited(answer());
    return lookup.future;
  }
}

class _FakeChannelsNotifier extends ChannelsNotifier {
  _FakeChannelsNotifier(this.channels);

  final Future<List<Channel>> channels;

  @override
  Future<List<Channel>> build() => channels;
}

class _DeferredInviteJoinRecovery implements InviteJoinRecovery {
  final Completer<String?> _result = Completer<String?>();
  final Completer<void> started = Completer<void>();

  @override
  Future<String?> ensureStarterChannels() {
    started.complete();
    return _result.future;
  }

  void complete(String? focusChannelId) => _result.complete(focusChannelId);

  void completeError(Object error) => _result.completeError(error);
}

class _CapturedDestination extends StatelessWidget {
  const _CapturedDestination({required this.channel, required this.link});

  final Channel channel;
  final BuzzDeepLink link;

  @override
  Widget build(BuildContext context) => const SizedBox();
}

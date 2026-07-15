import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/deep_link_dispatcher.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

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

class _FakePendingDeepLinkNotifier extends PendingDeepLinkNotifier {
  _FakePendingDeepLinkNotifier(this.link);

  final MessageDeepLink link;

  @override
  MessageDeepLink? build() => link;
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

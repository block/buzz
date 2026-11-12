import 'package:buzz/features/activity/activity_provider.dart';
import 'package:buzz/features/activity/feed_item.dart';
import 'package:buzz/features/activity/inbox_item.dart';
import 'package:buzz/features/activity/inbox_local_state_provider.dart';
import 'package:buzz/features/activity/inbox_unread_provider.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _ReadState extends ReadStateNotifier {
  @override
  ReadStateState build() => const ReadStateState.inert();

  void load(Map<String, int> contexts) => state = ReadStateState(
    isReady: true,
    pubkey: 'reader',
    contexts: contexts,
    version: state.version + 1,
  );
}

class _LocalState extends InboxLocalStateNotifier {
  @override
  InboxLocalState build() => const InboxLocalState.empty();
}

FeedItem item(String id, {String? channelId}) => FeedItem(
  id: id,
  kind: 9,
  pubkey: 'sender',
  content: 'An update',
  createdAt: 100,
  channelId: channelId,
  channelName: 'general',
  tags: const [],
  category: 'mention',
);

void main() {
  test('bell follows read markers and local unread overrides', () {
    final container = ProviderContainer(
      overrides: [
        inboxItemsProvider.overrideWithValue(
          buildInboxItems([item('mention', channelId: 'channel')]),
        ),
        readStateProvider.overrideWith(_ReadState.new),
        inboxLocalStateProvider.overrideWith(_LocalState.new),
      ],
    );
    addTearDown(container.dispose);
    expect(container.read(inboxHasUnreadProvider), isFalse);
    final read = container.read(readStateProvider.notifier) as _ReadState;
    read.load({});
    expect(container.read(inboxHasUnreadProvider), isTrue);
    read.load({'channel': 100});
    expect(container.read(inboxHasUnreadProvider), isFalse);
    final local = container.read(inboxLocalStateProvider.notifier);
    local.markUnread(['mention']);
    expect(container.read(inboxHasUnreadProvider), isTrue);
    local.clearUnread(['mention']);
    expect(container.read(inboxHasUnreadProvider), isFalse);
  });

  test('channel-less items clear when done and empty inbox has no dot', () {
    final container = ProviderContainer(
      overrides: [
        inboxItemsProvider.overrideWithValue(
          buildInboxItems([item('approval')]),
        ),
        readStateProvider.overrideWith(_ReadState.new),
        inboxLocalStateProvider.overrideWith(_LocalState.new),
      ],
    );
    addTearDown(container.dispose);
    (container.read(readStateProvider.notifier) as _ReadState).load({});
    expect(container.read(inboxHasUnreadProvider), isTrue);
    container.read(inboxLocalStateProvider.notifier).markDone('approval');
    expect(container.read(inboxHasUnreadProvider), isFalse);
    container.updateOverrides([
      inboxItemsProvider.overrideWithValue(const []),
      readStateProvider.overrideWith(_ReadState.new),
      inboxLocalStateProvider.overrideWith(_LocalState.new),
    ]);
    expect(container.read(inboxHasUnreadProvider), isFalse);
  });
}

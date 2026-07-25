import 'package:buzz/features/channels/read_state/message_read_state.dart';
import 'package:buzz/features/channels/read_state/read_state_provider.dart';
import 'package:flutter_test/flutter_test.dart';

ReadStateState _state(
  Map<String, int> contexts, {
  Set<String> forced = const {},
}) => ReadStateState(
  isReady: true,
  pubkey: 'pk',
  contexts: contexts,
  version: 1,
  locallyForcedChannelIds: forced,
);

void main() {
  const channelId = 'chan-1';
  const messageId = 'msg-1';

  group('effectiveMessageReadAt', () {
    test('is null with no markers', () {
      expect(
        effectiveMessageReadAt(
          _state(const {}),
          channelId: channelId,
          messageId: messageId,
        ),
        isNull,
      );
    });

    test('takes the newest of channel, message, and thread markers', () {
      final readState = _state(const {
        channelId: 100,
        'msg:$messageId': 300,
        'thread:root-1': 200,
      });
      expect(
        effectiveMessageReadAt(
          readState,
          channelId: channelId,
          messageId: messageId,
          threadRootId: 'root-1',
        ),
        300,
      );
    });

    test('ignores thread marker when no root is given', () {
      final readState = _state(const {'thread:root-1': 500});
      expect(
        effectiveMessageReadAt(
          readState,
          channelId: channelId,
          messageId: messageId,
        ),
        isNull,
      );
    });
  });

  group('isMessageUnread', () {
    test('unread when created after all markers', () {
      final readState = _state(const {channelId: 100});
      expect(
        isMessageUnread(
          readState,
          channelId: channelId,
          messageId: messageId,
          createdAt: 150,
        ),
        isTrue,
      );
    });

    test('read when covered by the channel marker', () {
      final readState = _state(const {channelId: 200});
      expect(
        isMessageUnread(
          readState,
          channelId: channelId,
          messageId: messageId,
          createdAt: 150,
        ),
        isFalse,
      );
    });

    test('read when covered by its own msg marker', () {
      final readState = _state(const {'msg:$messageId': 150});
      expect(
        isMessageUnread(
          readState,
          channelId: channelId,
          messageId: messageId,
          createdAt: 150,
        ),
        isFalse,
      );
    });

    test('thread reply covered by thread marker', () {
      final readState = _state(const {'thread:root-1': 400});
      expect(
        isMessageUnread(
          readState,
          channelId: channelId,
          messageId: messageId,
          createdAt: 350,
          threadRootId: 'root-1',
        ),
        isFalse,
      );
    });

    test('forced-unread channel wins over markers', () {
      final readState = _state(
        const {channelId: 1000},
        forced: const {channelId},
      );
      expect(
        isMessageUnread(
          readState,
          channelId: channelId,
          messageId: messageId,
          createdAt: 150,
        ),
        isTrue,
      );
    });
  });
}

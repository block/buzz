import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/single_level_thread.dart';
import 'package:buzz/features/channels/timeline_message.dart';

TimelineMessage message(String id, int time, {String? parent, String? root}) =>
    TimelineMessage(
      id: id,
      pubkey: 'author',
      createdAt: time,
      content: id,
      parentId: parent,
      rootId: root,
    );
void main() {
  test(
    'historical missing-parent replies flatten chronologically without mutation',
    () {
      final messages = [
        message('root', 1),
        message('z', 4, parent: 'missing', root: 'root'),
        message('a', 4, parent: 'root', root: 'root'),
        message('early', 2, parent: 'z', root: 'root'),
        message('other', 3, parent: 'another', root: 'another'),
      ];
      expect(flatThreadReplies(messages, 'root').map((m) => m.id), [
        'early',
        'a',
        'z',
      ]);
      expect(messages[1].parentId, 'missing');
    },
  );
  test(
    'legacy parent-only chains terminate even with cyclic malformed input',
    () {
      final messages = [
        message('a', 2, parent: 'root'),
        message('b', 3, parent: 'a'),
        message('root', 1, parent: 'b'),
      ];
      expect(flatThreadReplies(messages, 'root').map((m) => m.id), ['a', 'b']);
    },
  );
  test('reply context is separate from thread ancestry', () {
    const response = TimelineMessage(
      id: 'r',
      pubkey: 'author',
      createdAt: 3,
      content: 'response',
      parentId: 'root',
      rootId: 'root',
      tags: [
        ['reply-context', 'selected'],
      ],
    );
    expect(replyContextId(response), 'selected');
    expect(
      replyContextId(message('legacy', 3, parent: 'selected', root: 'root')),
      'selected',
    );
    expect(
      replyContextId(message('direct', 2, parent: 'root', root: 'root')),
      isNull,
    );
  });
}

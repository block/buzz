import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/shared/relay/nostr_models.dart';

void main() {
  const publisher =
      'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
  Channel channel({String? policy = publisher, bool member = false}) => Channel(
    id: 'updates',
    name: 'Updates',
    channelType: 'stream',
    visibility: 'open',
    description: '',
    createdBy: publisher,
    createdAt: DateTime.utc(2026),
    memberCount: 1,
    isMember: member,
    messagePublisher: policy,
  );
  test(
    'only the configured account gets a root composer, including existing members',
    () {
      expect(channel().canStartMessage(publisher), isTrue);
      expect(channel(member: true).canStartMessage('other'), isFalse);
      expect(channel().canStartMessage(null), isFalse);
      expect(
        channel(policy: null, member: true).canStartMessage('other'),
        isTrue,
      );
      expect(channel(policy: null).canStartMessage('other'), isFalse);
      expect(
        channel()
            .copyWith(archivedAt: DateTime.utc(2026))
            .canStartMessage(publisher),
        isFalse,
      );
    },
  );
  test(
    'policy survives snapshots and copies; absent and malformed metadata differ',
    () {
      expect(Channel.fromJson(channel().toJson()).messagePublisher, publisher);
      expect(channel().copyWith(name: 'renamed').messagePublisher, publisher);
      expect(
        channel()
            .mergeDetails(ChannelDetails.fromChannel(channel()))
            .messagePublisher,
        publisher,
      );
      expect(parseMessagePublisher([]), isNull);
      expect(
        parseMessagePublisher([
          ['message_publisher', publisher],
        ]),
        publisher,
      );
      expect(
        parseMessagePublisher([
          ['message_publisher', 'bad'],
        ]),
        '',
      );
      expect(
        parseMessagePublisher([
          ['message_publisher', publisher],
          ['message_publisher', publisher],
        ]),
        '',
      );
    },
  );
}

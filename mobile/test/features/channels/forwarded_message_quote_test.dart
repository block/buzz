import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/forwarded_message_quote.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/features/profile/user_cache_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';

import '../../helpers/widget_helpers.dart';

final _authorPubkey = 'a' * 64;
final _forwarderPubkey = 'b' * 64;

Map<String, dynamic> _originalJson() => {
  'id': 'c' * 64,
  'pubkey': _authorPubkey,
  'created_at': 1700000000,
  'kind': 40002,
  'tags': [
    ['h', 'src-channel'],
  ],
  'content': 'the original body',
  'sig': 'd' * 128,
};

List<List<String>> _forwardTags({String type = 'channel', String? fwdJson}) => [
  ['h', 'dest-channel'],
  ['fwd', fwdJson ?? jsonEncode(_originalJson())],
  ['k', '40002'],
  ['fwd-src', 'src-channel', type],
];

NostrEvent _forwardEvent({String note = '', List<List<String>>? tags}) =>
    NostrEvent(
      id: 'e' * 64,
      pubkey: _forwarderPubkey,
      createdAt: 1700000100,
      kind: EventKind.streamMessageForward,
      tags: tags ?? _forwardTags(),
      content: note,
      sig: 'f' * 128,
    );

Channel _sourceChannel({String visibility = 'open'}) => Channel(
  id: 'src-channel',
  name: 'general',
  channelType: 'stream',
  visibility: visibility,
  description: '',
  createdBy: _authorPubkey,
  createdAt: DateTime.utc(2024),
  memberCount: 2,
);

class _FakeChannelsNotifier extends ChannelsNotifier {
  final List<Channel> _channels;
  _FakeChannelsNotifier(this._channels);

  @override
  Future<List<Channel>> build() async => _channels;
}

class _FakeUserCacheNotifier extends UserCacheNotifier {
  final Map<String, UserProfile> _users;
  _FakeUserCacheNotifier(this._users);

  @override
  Map<String, UserProfile> build() => _users;
}

Future<void> _pumpQuote(
  WidgetTester tester,
  ForwardInfo forward, {
  List<Channel> channels = const [],
}) async {
  await tester.pumpWidget(
    WidgetHelpers.testable(
      overrides: [
        channelsProvider.overrideWith(() => _FakeChannelsNotifier(channels)),
        userCacheProvider.overrideWith(
          () => _FakeUserCacheNotifier({
            _authorPubkey: UserProfile(
              pubkey: _authorPubkey,
              displayName: 'Alice',
            ),
          }),
        ),
      ],
      child: ForwardedMessageQuote(forward: forward),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  group('ForwardInfo.fromTags', () {
    test('parses a valid forward', () {
      final info = ForwardInfo.fromTags(_forwardTags());
      expect(info, isNotNull);
      expect(info!.original.pubkey, _authorPubkey);
      expect(info.original.content, 'the original body');
      expect(info.original.kind, 40002);
      expect(info.sourceChannelId, 'src-channel');
      expect(info.sourceType, ForwardSourceType.channel);
    });

    test('parses private and dm source types', () {
      expect(
        ForwardInfo.fromTags(_forwardTags(type: 'private'))!.sourceType,
        ForwardSourceType.private,
      );
      expect(
        ForwardInfo.fromTags(_forwardTags(type: 'dm'))!.sourceType,
        ForwardSourceType.dm,
      );
    });

    test('returns null for malformed inputs', () {
      // Unparseable JSON.
      expect(ForwardInfo.fromTags(_forwardTags(fwdJson: 'not json')), isNull);
      // Wrong JSON shape.
      expect(ForwardInfo.fromTags(_forwardTags(fwdJson: '[1,2]')), isNull);
      // Missing fields in the embedded event.
      expect(ForwardInfo.fromTags(_forwardTags(fwdJson: '{"id":1}')), isNull);
      // Unknown source type label.
      expect(ForwardInfo.fromTags(_forwardTags(type: 'bogus')), isNull);
      // Missing fwd tag entirely.
      expect(
        ForwardInfo.fromTags([
          ['h', 'dest-channel'],
          ['fwd-src', 'src-channel', 'channel'],
        ]),
        isNull,
      );
    });
  });

  group('formatTimeline forwards', () {
    test('forward with note keeps the note and attaches forward info', () {
      final messages = formatTimeline([_forwardEvent(note: 'check this out')]);
      expect(messages, hasLength(1));
      expect(messages.first.content, 'check this out');
      expect(messages.first.pubkey, _forwarderPubkey);
      expect(messages.first.forward, isNotNull);
      expect(messages.first.forward!.original.content, 'the original body');
    });

    test('forward without note has empty content and forward info', () {
      final messages = formatTimeline([_forwardEvent()]);
      expect(messages, hasLength(1));
      expect(messages.first.content, isEmpty);
      expect(messages.first.forward, isNotNull);
    });

    test('malformed fwd tag falls back to a normal message', () {
      final messages = formatTimeline([
        _forwardEvent(
          note: 'just the note',
          tags: _forwardTags(fwdJson: '{broken'),
        ),
      ]);
      expect(messages, hasLength(1));
      expect(messages.first.content, 'just the note');
      expect(messages.first.forward, isNull);
    });
  });

  group('ForwardedMessageQuote', () {
    testWidgets('open source shows channel name, author, and body', (
      tester,
    ) async {
      final forward = ForwardInfo.fromTags(_forwardTags())!;
      await _pumpQuote(tester, forward, channels: [_sourceChannel()]);

      expect(find.text('Forwarded from #general'), findsOneWidget);
      expect(find.text('Alice'), findsOneWidget);
      expect(
        find.textContaining('the original body', findRichText: true),
        findsOneWidget,
      );
    });

    testWidgets('unknown open source falls back to a generic label', (
      tester,
    ) async {
      final forward = ForwardInfo.fromTags(_forwardTags())!;
      await _pumpQuote(tester, forward);

      expect(find.text('Forwarded from a channel'), findsOneWidget);
    });

    testWidgets('private source shows anonymous channel label', (tester) async {
      final forward = ForwardInfo.fromTags(_forwardTags(type: 'private'))!;
      await _pumpQuote(
        tester,
        forward,
        channels: [_sourceChannel(visibility: 'private')],
      );

      expect(find.text('Forwarded from a private channel'), findsOneWidget);
      expect(find.textContaining('#general'), findsNothing);
    });

    testWidgets('dm source shows direct-message label', (tester) async {
      final forward = ForwardInfo.fromTags(_forwardTags(type: 'dm'))!;
      await _pumpQuote(tester, forward);

      expect(find.text('Forwarded from a direct message'), findsOneWidget);
    });
  });
}

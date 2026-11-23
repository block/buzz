import 'package:buzz/features/channels/channel_event_order.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  final comparators = {
    'channel': compareChannelTimelineEventsChronologically,
    'thread': compareThreadRepliesChronologically,
  };
  for (final entry in comparators.entries) {
    final compare = entry.value;
    test('${entry.key} orders all millisecond offsets within each second', () {
      final events = [
        _event('next', createdAt: 101),
        for (var ms = 999; ms >= 0; ms--)
          _event(
            '${999 - ms}',
            tags: [
              ['ms', '$ms'],
            ],
            content: '$ms',
          ),
        _event(
          'previous',
          createdAt: 99,
          tags: [
            ['ms', '999'],
          ],
        ),
      ]..sort(compare);
      expect(events.map((event) => event.content), [
        'previous',
        for (var ms = 0; ms < 1000; ms++) '$ms',
        'next',
      ]);
    });

    test('${entry.key} validates the first ms tag like desktop', () {
      final zero = _event(
        'same',
        tags: [
          ['ms', '0'],
        ],
      );
      for (final tags in <List<List<String>>>[
        [],
        [[]],
        [
          ['ms'],
        ],
        for (final value in [
          '',
          '-1',
          '1000',
          '100900',
          '1.5',
          '1e2',
          '+1',
          ' 1',
          '1 ',
          '01',
          '000',
          '007',
          '12a',
          'NaN',
        ])
          [
            ['ms', value],
          ],
        [
          ['ms'],
          ['ms', '900'],
        ],
        [
          ['ms', 'bad'],
          ['ms', '900'],
        ],
        [
          ['ms', '0'],
          ['ms', '900'],
        ],
      ]) {
        expect(compare(_event('same', tags: tags), zero), 0, reason: '$tags');
      }
      expect(
        compare(
          _event(
            'same',
            tags: [
              [],
              ['other', '1'],
              ['ms', '100'],
              ['ms', '900'],
            ],
          ),
          _event(
            'same',
            tags: [
              ['ms', '100'],
            ],
          ),
        ),
        0,
      );
    });

    test(
      '${entry.key} preserves its id tie-break at equal effective times',
      () {
        for (final tags in <List<List<String>>>[
          [],
          [
            ['ms', '500'],
          ],
        ]) {
          final events = [_event('a', tags: tags), _event('z', tags: tags)]
            ..sort(compare);
          expect(
            events.map((event) => event.id),
            entry.key == 'channel' ? ['z', 'a'] : ['a', 'z'],
          );
        }
      },
    );
  }
}

NostrEvent _event(
  String id, {
  int createdAt = 100,
  List<List<String>> tags = const [],
  String? content,
}) => NostrEvent(
  id: id,
  pubkey: 'alice',
  createdAt: createdAt,
  kind: EventKind.streamMessageV2,
  tags: tags,
  content: content ?? id,
  sig: '',
);

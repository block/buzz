import 'package:buzz/features/channels/message_content/preview_text.dart';
import 'package:buzz/features/channels/message_media.dart';
import 'package:flutter_test/flutter_test.dart';

const _alice =
    'a11ce00000000000000000000000000000000000000000000000000000000000';

const _channelId = '0b5e2c3a-1d4f-4a6b-9c8d-7e6f5a4b3c2d';
const _channelLink = 'buzz://channel/$_channelId';

List<PreviewPiece> _pieces(
  String content, {
  int maxLines = 2,
  Map<String, Set<String>> mentionBindings = const {},
  Map<String, String> mentionNames = const {},
  Map<String, String> customEmoji = const {},
  Map<String, String> channelNames = const {},
}) => messagePreviewPieces(
  content,
  maxLines: maxLines,
  mentionBindings: mentionBindings,
  mentionNames: mentionNames,
  customEmoji: customEmoji,
  channelNames: channelNames,
);

String _text(String content, {int maxLines = 2}) => [
  for (final piece in _pieces(content, maxLines: maxLines))
    if (piece is PreviewText) piece.text else '<$piece>',
].join();

void main() {
  group('markup', () {
    for (final (name, source, expected) in [
      ('plain text', 'hello world', 'hello world'),
      ('heading', '## Release notes', 'Release notes'),
      ('block quote', '> > quoted', 'quoted'),
      ('bullet', '- item', 'item'),
      ('task', '- [x] done', 'done'),
      ('ordered item', '12. twelfth', 'twelfth'),
      ('bold', 'a **bold** word', 'a bold word'),
      ('italic', 'an *italic* and _under_ word', 'an italic and under word'),
      ('strike', '~~gone~~ now', 'gone now'),
      ('snake_case', 'call my_func_name now', 'call my_func_name now'),
      ('lone asterisk', '2 * 3 = 6', '2 * 3 = 6'),
      ('inline code', 'run `**not bold**` here', 'run **not bold** here'),
      ('unpaired backtick', 'a ` b **c**', 'a ` b c'),
      ('double-backtick code', 'x ``**a** `b` @c`` y', 'x **a** `b` @c y'),
      ('unmatched backtick run', 'a `` b `c` **d**', 'a `` b c d'),
      ('padded code span', '`` `x` ``', '`x`'),
      (
        'encoded label brackets',
        '[report &#91;Q4&#93;](https://x.y)',
        'report [Q4]',
      ),
      ('link', 'see [the docs](https://example.com)', 'see the docs'),
      ('bare link label', '[https://x.y](https://x.y)', 'https://x.y'),
      ('rule', 'above\n---\nbelow', 'above\nbelow'),
      ('table', '| a | b |\n|---|:-:|\n| 1 | 2 |', 'a  b\n1  2'),
      ('blank lines', 'one\n\n\ntwo', 'one\ntwo'),
      ('underline', 'an <u>important</u> note', 'an important note'),
      ('escaped emphasis', r'\*literal\* and \_x\_', '*literal* and _x_'),
      ('escaped backslash', r'a \\ b', r'a \ b'),
      ('escaped link opener', r'\[docs](https://x)', '[docs](https://x)'),
      ('escaped backticks', r'\`literal\` and `code`', '`literal` and code'),
    ]) {
      test(name, () => expect(_text(source), expected));
    }

    test('keeps fenced code literally without its fences', () {
      expect(_text('```dart\n**x** @y\n```\nafter'), '**x** @y\nafter');
    });

    test('a fence closes only at a bare fence line', () {
      expect(_text('```\n```not a close\n```\nafter'), '```not a close\nafter');
    });
  });

  group('line limit', () {
    test('the source prefix ends with the last line the preview reads', () {
      const content = '# a\n\n```\ncode\n```\nb\nc\nd\ne';
      expect(
        messagePreviewSource(content, maxLines: 2),
        '# a\n\n```\ncode\n```\nb',
      );
      expect(_text(messagePreviewSource(content, maxLines: 2)), 'a\ncode\nb');
    });

    test('the source prefix of a short message is the message', () {
      expect(messagePreviewSource('one\ntwo', maxLines: 2), 'one\ntwo');
      expect(messagePreviewSource('', maxLines: 2), '');
    });

    test('reads only maxLines + 1 kept lines', () {
      final content = List.generate(1000, (i) => 'line $i').join('\n');
      expect(_text(content), 'line 0\nline 1\nline 2');
    });

    test('dropped lines do not count', () {
      expect(_text('# a\n\n---\n- b\nc\nd'), 'a\nb\nc');
    });

    test('stops reading after a long run of lines that do not render', () {
      final content = '\n' * 1000000 + 'late';
      expect(_text(content), '');
      expect(
        messagePreviewSource(content, maxLines: 2).length,
        lessThan(20000),
      );
    });

    test('caps a very long line, and reads only its start', () {
      expect(_text('x' * 5000).length, 2000);
      expect(
        messagePreviewSource('x' * 128000, maxLines: 2).length,
        lessThanOrEqualTo(4000),
      );
    });
  });

  group('tokens', () {
    test('a bound mention becomes a chip with its display name', () {
      expect(
        _pieces(
          'hi @alice!',
          mentionBindings: {
            'alice': {_alice},
          },
          mentionNames: {_alice: 'Alice Smith'},
        ),
        [
          const PreviewText('hi '),
          const PreviewMention(
            pubkey: _alice,
            label: 'Alice Smith',
            semanticsLabel: 'Alice Smith',
          ),
          const PreviewText('!'),
        ],
      );
    });

    test('an ambiguous mention stays text', () {
      expect(
        _pieces(
          'hi @sam',
          mentionBindings: {
            'sam': {'a', 'b'},
          },
        ),
        [const PreviewText('hi @sam')],
      );
    });

    test('a mention inside emphasis or a link label is still a chip', () {
      final pieces = _pieces(
        '**@alice** [@alice](https://x.y)',
        mentionBindings: {
          'alice': {_alice},
        },
      );
      expect(pieces.whereType<PreviewMention>(), hasLength(2));
    });

    test('an escaped mention stays text', () {
      expect(
        _pieces(
          r'hi \@alice',
          mentionBindings: {
            'alice': {_alice},
          },
        ),
        [const PreviewText('hi @alice')],
      );
    });

    test('a mention in inline code stays text', () {
      expect(
        _pieces(
          '`@alice`',
          mentionBindings: {
            'alice': {_alice},
          },
        ),
        [const PreviewText('@alice')],
      );
    });

    test('known custom emoji become images, unknown stay text', () {
      expect(
        _pieces(
          ':nope:wave: :Wave:',
          customEmoji: {'wave': 'https://e/wave.png'},
        ),
        [
          const PreviewText(':nope'),
          const PreviewEmoji(shortcode: 'wave', url: 'https://e/wave.png'),
          const PreviewText(' '),
          const PreviewEmoji(shortcode: 'wave', url: 'https://e/wave.png'),
        ],
      );
    });

    test('channel tokens become chips, but not inside words or URLs', () {
      expect(
        _pieces(
          'see #general, a#b and x.y/#z',
          channelNames: {'general': 'c1'},
        ),
        [
          const PreviewText('see '),
          const PreviewChannel('general'),
          const PreviewText(', a#b and x.y/#z'),
        ],
      );
    });

    test('only known channels become chips once channels have loaded', () {
      expect(
        _pieces(
          'took over #8700, see #general and #8774\'s #gen',
          channelNames: {'general': 'c1'},
        ),
        [
          const PreviewText('took over #8700, see '),
          const PreviewChannel('general'),
          const PreviewText(" and #8774's #gen"),
        ],
      );
      // A known prefix doesn't claim a longer unknown name; the longest known
      // name wins when both are known.
      expect(_pieces('#eng-ios', channelNames: {'eng': 'c1'}), [
        const PreviewText('#eng-ios'),
      ]);
      expect(
        _pieces('#eng-ios', channelNames: {'eng': 'c1', 'eng-ios': 'c2'}),
        [const PreviewChannel('eng-ios')],
      );
      expect(_pieces('see #8700'), [
        const PreviewText('see '),
        const PreviewChannel('8700'),
      ]);
    });

    test('channel tokens end at punctuation that follows a word', () {
      for (final (before, after) in [
        ('', "'s plan"),
        ('', '\u2019s plan'),
        ('"', '"'),
        ('\u201C', '\u201D'),
        ('\u2018', '\u2019'),
        ('\u00AB', '\u00BB'),
        ('', '\u2026'),
        ('', '\u2014next'),
        ('', '\u2013next'),
      ]) {
        expect(
          _pieces('see $before#general$after', channelNames: {'general': 'c1'}),
          [
            PreviewText('see $before'),
            const PreviewChannel('general'),
            PreviewText(after),
          ],
          reason: '$before#general$after',
        );
      }
    });

    test('a buzz channel link becomes a chip with the channel name', () {
      expect(
        _pieces(
          '[$_channelLink]($_channelLink)',
          channelNames: {'general': _channelId},
        ),
        [const PreviewChannel('general')],
      );
    });

    test('message and repo links that show their URL get a short label', () {
      final message = 'buzz://message?channel=$_channelId&id=${'ab' * 32}';
      const repo = 'buzz://repo?owner=$_alice&d=buzz';
      expect(
        _pieces(
          '[$message]($message) [$repo]($repo) [named]($message)',
          channelNames: {'general': _channelId},
        ),
        [const PreviewText('general · abababab buzz named')],
      );
    });

    test('media links become their kind', () {
      expect(_pieces('look ![](https://x.y/a.png) [clip](https://x.y/b.mp4)'), [
        const PreviewText('look '),
        const PreviewMedia(MessageMediaKind.image),
        const PreviewText(' '),
        const PreviewMedia(MessageMediaKind.video),
      ]);
    });
  });
}

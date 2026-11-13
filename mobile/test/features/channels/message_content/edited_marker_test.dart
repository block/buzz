import 'package:buzz/features/channels/message_content/edited_marker.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test(
    'the edited slot joins prose and nothing gpt_markdown could restyle',
    () {
      for (final (markdown, expected) in [
        ('Hi how are ya.', 'Hi how are ya. $editedSlot'),
        ('Trailing space  \n', 'Trailing space $editedSlot'),
        ('- one\n- two', '- one\n- two $editedSlot'),
        ('> quoted', '> quoted $editedSlot'),
        ('# Title\nbody', '# Title\nbody $editedSlot'),
        ('Visit https://x.io today', 'Visit https://x.io today $editedSlot'),
        ('#######', '####### $editedSlot'),
        ('a \\| b', 'a \\| b $editedSlot'),
        ('Read [docs](x) now', 'Read [docs](x) now $editedSlot'),
        ('done :)', 'done :) $editedSlot'),
        (')', ') $editedSlot'),
        ('see ://x', 'see ://x $editedSlot'),
        ('see 1://x', 'see 1://x $editedSlot'),
      ]) {
        expect(withEditedSlot(markdown), expected, reason: markdown);
      }
      for (final markdown in [
        '',
        '```npm install```',
        'Run this:\n```npm install```',
        'Intro\n\n```\ncode\n```\nafter',
        '````\ncode',
        '~~~\ncode\n~~~',
        'Run this:\n~~~\ncode',
        'wavy\n~~~',
        'Hidden <!-- note',
        '<!-- note -->\nafter',
        '    indented code',
        'hi\n\tcode',
        '| a | b |\n| - | - |\n| 1 | 2 |',
        'a or b |',
        'a | b\n--|--\nc | d',
        'ok\n1)',
        'Above\n\n---',
        'Above\n\n* * *',
        'Title\n===',
        'Title\n--',
        r'$$x^2$$',
        for (final marker in [
          '-',
          '*',
          '+',
          '#',
          '###',
          '>',
          '1.',
          '[ ]',
          '[x]',
        ])
          'ok\n$marker',
        'pick\n( )',
        '# Heading',
        '### Notes  ',
        'ok\n\nhttps://example.com/file.pdf',
        'see https://example.com.',
        'Read [the docs](https://x.io)',
        'Read [the docs](https://x.io) ',
        '[a](b) [c](d)',
        'go to(https://x.io',
        'see x+1.y-z://host',
        'buzz://message?channel=c&id=e',
        'kept $editedSlot as written',
      ]) {
        expect(withEditedSlot(markdown), isNull, reason: markdown);
      }
    },
  );

  test('a long authored last line takes linear time', () {
    // Each body defeats an unanchored pattern at every start position, which
    // took seconds at 40,000 characters and grows with the square.
    const n = 256 * 1024;
    for (final markdown in [
      '${'a' * n}:// x y',
      '${'](' * (n ~/ 2)}x y',
      '${'a:' * (n ~/ 2)}//',
      '${'|' * n} x',
      '${r'$' * n} x',
      '${'- ' * (n ~/ 2)}x',
    ]) {
      final timer = Stopwatch()..start();
      withEditedSlot(markdown);
      expect(
        timer.elapsedMilliseconds,
        lessThan(500),
        reason: markdown.substring(0, 8),
      );
    }
  });
}

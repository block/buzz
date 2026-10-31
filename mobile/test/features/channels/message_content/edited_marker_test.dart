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
        ('# Heading', '# Heading $editedSlot'),
        ('wavy\n~~~', 'wavy\n~~~ $editedSlot'),
        ('ok\n--', 'ok\n-- $editedSlot'),
        ('a | b', 'a | b $editedSlot'),
      ]) {
        expect(withEditedSlot(markdown), expected, reason: markdown);
      }
      for (final markdown in [
        '',
        '```npm install```',
        'Run this:\n```npm install```',
        'Intro\n\n```\ncode\n```\nafter',
        '````\ncode',
        '    indented code',
        'hi\n\tcode',
        '| a | b |\n| - | - |\n| 1 | 2 |',
        'a or b |',
        'Above\n\n---',
        'Above\n\n* * *',
        'Title\n===',
        r'$$x^2$$',
        r'\[x^2\]',
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
      ]) {
        expect(withEditedSlot(markdown), isNull, reason: markdown);
      }
    },
  );
}

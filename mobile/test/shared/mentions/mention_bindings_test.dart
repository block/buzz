import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/mentions/mention_bindings.dart';

void main() {
  final first = 'a' * 64;
  final second = 'b' * 64;
  test('longest occurrences block shorter and interior recipients', () {
    expect(
      mentionOccurrences('@Scout ($second) 2', [
        'Scout',
        'Scout ($second)',
        'Scout ($second) 2',
      ]).single.label,
      'Scout ($second) 2',
    );
    expect(mentionOccurrences('@A @B', ['A @B', 'B']).single.label, 'A @B');
    expect(mentionOccurrences('mail@Scout', ['Scout']), isEmpty);
    // A mention opens where a query does, so a pick after a bracket binds.
    for (final bracket in ['(', '[', '{']) {
      expect(
        mentionOccurrences('see $bracket@Scout', ['Scout']).single.start,
        5,
        reason: 'after $bracket',
      );
    }
    expect(mentionOccurrences('see {@Scout}', ['Scout']).single.end, 11);
    expect(mentionOccurrences('@Scout ($second)', ['Scout']), isEmpty);
    expect(
      mentionOccurrences('@Scout ($second) 2', ['Scout ($second)']),
      isEmpty,
    );
  });
  test('a label ends at the composer suffix boundary', () {
    // Punctuation, either apostrophe, a hyphen or a slash follow the name as
    // text, and so do underscores closing Markdown emphasis; a letter, mark
    // or digit continues it (`@Bobby`), also through underscores.
    for (final suffix in [
      "'s idea",
      '’s idea',
      '-bot',
      '/Alice',
      '.',
      '_',
      '__',
      '_ x',
    ]) {
      final found = mentionOccurrences('@Scout$suffix', ['Scout']);
      expect(found.single.end, 6, reason: 'after $suffix');
    }
    // Sent italic and bold mentions keep the whole name (`_@Kenny Lopez_`
    // never falls back to the first-name alias).
    for (final text in ['_@Kenny Lopez_', '__@Kenny Lopez__ ok']) {
      expect(
        mentionOccurrences(text, ['Kenny', 'Kenny Lopez']).single.label,
        'Kenny Lopez',
        reason: text,
      );
    }
    for (final text in ['@Scout_y', '@Scout__1']) {
      expect(mentionOccurrences(text, ['Scout']), isEmpty, reason: text);
    }
    for (final text in ['@Scouty', '@Scout1', '@Scoutē']) {
      expect(mentionOccurrences(text, ['Scout']), isEmpty, reason: text);
    }
    // A reservation suffix ends at the same boundary, so the shorter label
    // still never claims `@Scout (<key>) 2's`.
    expect(
      mentionOccurrences("@Scout ($second) 2's", ['Scout ($second)']),
      isEmpty,
    );
    expect(
      mentionOccurrences('@Scout ($second) 2x', ['Scout ($second)']).single.end,
      '@Scout ($second)'.length,
    );
  });
  test('a typed name notifies only at the old end boundary, outside a link '
      'label', () {
    for (final (text, notifies) in [
      ('@Alice', true),
      ('@Alice hi', true),
      ('@Alice, hi', true),
      ('see (@Alice)', true),
      ('ping [@Alice] now', true),
      ('_@Alice_', true),
      ('**@Alice**', true),
      ('||@Alice||', true),
      ('mail @Alice@example.org', false),
      ("@Alice's idea", false),
      ('@Alice-bot', false),
      ('@Alice/Bob', false),
      ('[@Alice](https://example.org/a)', false),
      ('[hi @Alice there](https://example.org/a)', false),
      ('[link](https://example.org/a) @Alice', true),
    ]) {
      final range = mentionOccurrences(text, ['Alice']).single;
      expect(
        typedMentionNotifies(text, range.start, range.end),
        notifies,
        reason: text,
      );
    }
  });
  // Historical plain denial and both tag orders run through channel/thread UI.
  test('ordinary ambiguity and qualification require signed authority', () {
    expect(
      renderedMentionBindings('@Scout', {
        first: 'Scout',
        second: 'Scout',
      })['scout'],
      {first, second},
    );
    expect(
      renderedMentionBindings('@Scout ($second)', {
        first: 'Scout',
      })['scout ($second)'],
      isEmpty,
    );
    expect(
      renderedMentionBindings(
        '@Old ($second)',
        {second: 'New'},
        [second],
      )['old ($second)'],
      {second},
    );
  });
}

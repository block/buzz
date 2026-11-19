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
    // text; a letter, mark, digit or underscore continues it (`@Bobby`).
    for (final suffix in ["'s idea", '’s idea', '-bot', '/Alice', '.', '_']) {
      final found = mentionOccurrences('@Scout$suffix', ['Scout']);
      if (suffix == '_') {
        expect(found, isEmpty, reason: 'after $suffix');
      } else {
        expect(found.single.end, 6, reason: 'after $suffix');
      }
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

import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/mentions/mention_bindings.dart';

void main() {
  final first = 'a' * 64;
  final second = 'b' * 64;
  test('qualification is case-insensitive and collision-safe', () {
    final bindings = {'Scout': first, 'Scout ($second)': first};
    expect(selectedMentionLabel('scout', first, bindings), 'scout');
    expect(
      selectedMentionLabel('Scout', second, bindings),
      'Scout ($second) 2',
    );
  });
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
    expect(mentionOccurrences('@Scout ($second)', ['Scout']), isEmpty);
    expect(
      mentionOccurrences('@Scout ($second) 2', ['Scout ($second)']),
      isEmpty,
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

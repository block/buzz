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
  test('a namesake qualifies earlier plain selections too', () {
    expect(
      selectedMentionLabel('Scout', second, {'Scout': first}),
      'Scout ($second)',
    );
    expect(
      selectedMentionLabel('Scout', first, {'Scout ($second)': second}),
      'Scout ($first)',
    );
    expect(selectedMentionLabel('Other', second, {'Scout': first}), 'Other');
    final renames = namesakeRequalifications('scout', second, {
      'Scout': first,
      'Other': first,
    });
    expect(renames, {'Scout': 'Scout ($first)'});
    expect(namesakeRequalifications('Scout', first, {'Scout': first}), isEmpty);
    // Offsets before the query shift; a query at the offset is left to splice.
    const text = '@Scout and @Scout @Scout';
    final renamed = renameMentions(text, ['Scout'], renames, 18);
    expect(renamed.text, '@Scout ($first) and @Scout ($first) @Scout');
    expect(renamed.offset, 18 + 2 * ' ($first)'.length);
    expect(renamed.renamed, {'Scout'});
    // Authored spelling stays; an excluded occurrence is left as typed.
    final kept = renameMentions(
      '@SCOUT `x @Scout y` @',
      ['Scout'],
      renames,
      20,
      excluded: (start, end) => start == 10,
    );
    expect(kept.text, '@SCOUT ($first) `x @Scout y` @');
    expect(kept.offset, 20 + ' ($first)'.length);
    expect(kept.kept, {'Scout'});
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

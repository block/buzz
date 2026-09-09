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
  test('multiple historical bases remain unbound despite current aliases', () {
    final bindings = renderedMentionBindings(
      '@Scout @Scout ($second) @Zed @Zed ($first)',
      {
        first: 'Old Zed',
        second: 'Old Scout',
        'c' * 64: 'Scout',
        'd' * 64: 'Zed',
      },
    );
    expect(bindings['scout'], isEmpty);
    expect(bindings['zed'], isEmpty);
    expect(bindings['scout ($second)'], {second});
    expect(bindings['zed ($first)'], {first});
  });
  test('qualified identity does not authorize an inferred plain namesake', () {
    for (final names in [
      {'a': 'Scout', second: 'Scout'},
      {second: 'Scout', 'a': 'Scout'},
    ]) {
      final bindings = renderedMentionBindings(
        '@Scout @Scout ($second)',
        names,
      );
      expect(bindings['scout'], isEmpty);
      expect(bindings['scout ($second)'], {second});
    }
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
      renderedMentionBindings('@Old ($second)', {
        second: 'New',
      })['old ($second)'],
      {second},
    );
  });
}

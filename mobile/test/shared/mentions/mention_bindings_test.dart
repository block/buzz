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
  test('names compare as a case-insensitive Unicode pattern compares them', () {
    // The lookup replaced a pattern built from each label. `toLowerCase`
    // differs from that pattern on these, so each is pinned.
    for (final (text, label, found) in [
      ('@SCOUT', 'scout', true),
      ('@ſcout', 'scout', true), // long s
      ('@Kenny', 'kenny', true), // Kelvin sign
      ('@ΟΔΥΣΣΕΥΣ', 'οδυσσευς', true), // final sigma
      ('@ΟΔΥΣΣΕΥσ', 'οδυσσευς', true),
      ('@STRAẞE', 'straße', true),
      ('@strasse', 'straße', false),
      ('@ılker', 'ilker', false), // dotless i is its own letter
      ('@ILKER', 'ılker', false),
      ('@İlker', 'ilker', false),
      ('@İlker', 'i̇lker', false), // what toLowerCase makes of İ
      ('@İLKER', 'İlker', true),
      ('@ᲐᲜᲐ', 'ანა', true), // Georgian capitals
      ('@\u{1E900}\u{1E901}', '\u{1E922}\u{1E923}', true), // Adlam
      ('@JOSÉ', 'josé', false), // no normalization
    ]) {
      expect(
        mentionOccurrences('hi $text.', [label]).map((range) => range.label),
        found ? [label] : isEmpty,
        reason: '$text against $label',
      );
      expect(
        mentionOccurrences(
          'hi $text.',
          const [],
          known: MentionLabelIndex([label]),
        ),
        hasLength(found ? 1 : 0),
        reason: '$text against indexed $label',
      );
    }
    // Ranges are offsets in the text as written, also after a letter whose
    // lower case is longer than it is.
    final range = mentionOccurrences('İİ @İLKER!', ['İlker']).single;
    expect((range.start, range.end), (3, 9));
    // A label given with the call wins over an indexed one equal to it.
    expect(
      mentionOccurrences('@Scout', [
        'scout',
      ], known: MentionLabelIndex(['SCOUT', 'Scout Marie'])).single.label,
      'scout',
    );
  });
  test('a scan looks up no more names in a roster of 7,000 than of 70', () {
    String name(int i) => '${['Sam', 'José', 'Mary-Jane'][i % 3]} Lopez $i';
    final small = MentionLabelIndex([for (var i = 0; i < 70; i++) name(i)]);
    final large = MentionLabelIndex([
      for (var i = 0; i < 7000; i++) name(i),
      // One very long name costs one more length to try, not one per unit.
      'x' * 5000,
    ]);
    int lookups(String text, MentionLabelIndex known, List<String> expected) {
      final before = mentionScanLookups;
      expect(
        mentionOccurrences(text, const [], known: known).map((r) => r.label),
        expected,
      );
      return mentionScanLookups - before;
    }

    expect(lookups('no one is named here', large, []), 0);
    expect(lookups('mail@Sam Lopez 3 is an address', large, []), 0);
    const text = 'hi @sam lopez 3, (@JOSÉ LOPEZ 4) and _@Mary-Jane Lopez 2_';
    const found = ['Sam Lopez 3', 'José Lopez 4', 'Mary-Jane Lopez 2'];
    final work = lookups(text, large, found);
    // Three `@` signs, each tried at a handful of name lengths.
    expect(work, lessThan(40));
    expect(work, lessThanOrEqualTo(lookups(text, small, found) + 3 * 6));
    // Labels given with the call are indexed on that call: still no pattern
    // per name. Generous: about 20 ms, against five seconds with one.
    final clock = Stopwatch()..start();
    expect(
      mentionOccurrences(text, [for (var i = 0; i < 7000; i++) name(i)]),
      hasLength(3),
    );
    expect(clock.elapsed, lessThan(const Duration(seconds: 1)));
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

import 'package:buzz/features/channels/message_content/markdown_link_mentions.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('finds the mentions a bubble reads as code or part of a link', () {
    // Each row: text, and whether each `@Bob` in it is inside code or a link.
    const rows = <(String, List<bool>)>[
      ('hi @Bob', [false]),
      ('[@Bob](https://example.org/bob)', [true]),
      ('[see @Bob here](https://example.org) and @Bob', [true, false]),
      ('[site](https://example.org/(@Bob)) @Bob', [true, false]),
      ('[site](https://example.org "by @Bob") @Bob', [true, false]),
      ('![@Bob](https://example.org/bob.png) @Bob', [true, false]),
      ('<https://example.org/(@Bob)> @Bob', [true, false]),
      ('https://example.org/?q=@Bob @Bob', [true, false]),
      ('[@Bob][ref] @Bob\n\n[ref]: https://example.org', [true, false]),
      ('@Bob [x][ref]\n\n[ref]: https://example.org "by @Bob"', [false, true]),
      // Not links: the name stays a mention.
      (r'\[@Bob\](https://example.org/bob)', [false]),
      ('[@Bob] (@Bob) {@Bob}', [false, false, false]),
      ('[@Bob](not closed', [false]),
      ('_@Bob_ **@Bob** ~~@Bob~~', [false, false, false]),
      ('- [ ] @Bob\n- @Bob\n\n> @Bob\n\n# @Bob', [false, false, false, false]),
      (
        '| a | b |\n| - | - |\n| @Bob | [@Bob](https://example.org) |',
        [false, true],
      ),
      // A reference and its definition are the link they form; a name
      // beside them is still a mention.
      ('see [@Bob]\n\n[@Bob]: https://example.org/bob', [true, true]),
      (
        '@Bob [@Bob] [x][@Bob] [@Bob][] @Bob\n\n'
            '[@Bob]: https://example.org/bob\n\n@Bob',
        [false, true, true, true, false, true, false],
      ),
      // The bubble lifts `@Bob` and `@bob` out as different text, so a
      // reference in one case does not find a definition in another.
      ('[@Bob] @bob\n\n[@bob]: https://example.org/bob', [false, false, true]),
      // A definition nothing refers to is still not prose; a reference with
      // no definition is text.
      ('[@Bob]: https://example.org/bob', [true]),
      ('see [@Bob] and @Bob', [false, false]),
      // Code, as the bubble reads it, is not prose.
      ('`[@Bob](https://example.org)` @Bob', [true, false]),
      ('`@Bob` @Bob', [true, false]),
      ('``@Bob`` and ``a ` @Bob`` @Bob', [true, true, false]),
      ('```\n@Bob\n```\n@Bob', [true, false]),
      ('~~~\n@Bob\n~~~\n@Bob', [true, false]),
      ('````\n@Bob\n```\n@Bob\n````\n@Bob', [true, true, false]),
      ('@Bob\n```\n@Bob unclosed', [false, true]),
      ('@Bob\n~~~\n@Bob unclosed', [false, true]),
      ('**`@Bob`** and `` @Bob', [true, false]),
    ];
    for (final (text, expected) in rows) {
      final mentions = [
        for (final match in RegExp(
          '@bob',
          caseSensitive: false,
        ).allMatches(text))
          (start: match.start, end: match.end),
      ];
      expect(mentions, hasLength(expected.length), reason: text);
      final inside = mentionsInsideMarkdownLinks(text, mentions);
      expect(
        [for (var i = 0; i < mentions.length; i++) inside.contains(i)],
        expected,
        reason: text,
      );
    }
  });

  test('tells whole ranges apart and many ranges that share a label', () {
    // An identity link is replaced whole.
    const link = '[@Bob](nostr:npub1example)';
    const text = '$link ``$link`` [$link](https://example.org)';
    final links = [
      for (final match in link.allMatches(text))
        (start: match.start, end: match.end),
    ];
    expect(mentionsInsideMarkdownLinks(text, links, keep: 0), {1, 2});

    // Each of many same-label ranges is told apart: every third is code.
    final many = [
      for (var i = 0; i < 40; i++) i % 3 == 0 ? '``@Bob``' : '@Bob',
    ].join(' ');
    final ranges = [
      for (final match in '@Bob'.allMatches(many))
        (start: match.start, end: match.end),
    ];
    expect(mentionsInsideMarkdownLinks(many, ranges), {
      for (var i = 0; i < 40; i += 3) i,
    });
  });
}

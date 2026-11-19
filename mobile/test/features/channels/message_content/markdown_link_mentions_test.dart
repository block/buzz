import 'package:buzz/features/channels/message_content/markdown_link_mentions.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('finds the mentions a bubble reads as part of a link', () {
    // Each row: text, and whether each `@Bob` in it is inside a link.
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
      // Code is not a link; the composer's own code ranges handle it.
      ('`[@Bob](https://example.org)` @Bob', [false, false]),
    ];
    for (final (text, expected) in rows) {
      final mentions = [
        for (final match in '@Bob'.allMatches(text))
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
}

import 'package:buzz/features/channels/message_content/link_normalizer.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('attachment destinations', () {
    const media = 'https://relay.example/media/image.png';
    for (final prefix in ['!', '']) {
      test('$prefix preserves angle destinations and optional titles', () {
        for (final destination in ['<$media>', media, '<$media> "Title"']) {
          expect(
            normalizeMarkdownLinks('$prefix[label]($destination)'),
            '$prefix[label]($media)',
          );
        }
      });
    }
    for (final title in [
      '(Video title)',
      r'"A \"preview\""',
      r"'A \'preview\''",
      r'(A \(preview\))',
    ]) {
      for (final prefix in ['!', '']) {
        test('$prefix supports title $title', () {
          expect(
            normalizeMarkdownLinks('$prefix[label](<$media> $title)'),
            '$prefix[label]($media)',
          );
        });
      }
    }
    test('near-limit malformed openers complete without suffix rescans', () {
      // A generous wall-clock ceiling catches the original ~20 second freeze
      // while allowing slow CI hosts ample headroom for the linear scanner.
      for (final unit in ['[x](', '[', r'[x\[', r'[x\]']) {
        final input = unit * (256 * 1024 ~/ unit.length);
        final watch = Stopwatch()..start();
        final result = normalizeMarkdownLinks(input);
        watch.stop();
        expect(result == input, isTrue);
        expect(watch.elapsed, lessThan(const Duration(seconds: 2)));
      }
    });
    for (final (label, rendered) in [
      (r'report \[Q4\].pdf', 'report &#91;Q4&#93;.pdf'),
      (r'only \[ opening', 'only &#91; opening'),
      (r'only \] closing', 'only &#93; closing'),
      (
        r'backslash \\ and \[brackets\]',
        'backslash &#92; and &#91;brackets&#93;',
      ),
      ('nested [label]', 'nested &#91;label&#93;'),
    ]) {
      for (final prefix in ['!', '']) {
        test('$prefix preserves escaped label $label', () {
          expect(
            normalizeMarkdownLinks('$prefix[$label](<$media>)'),
            '$prefix[$rendered]($media)',
          );
        });
      }
    }
    test('URL and title brackets do not capture a later attachment label', () {
      expect(
        normalizeMarkdownLinks(
          '[first](<https://relay.example/file?q=[> "Title [") '
          r'[second \[Q4\]](<https://relay.example/file.pdf>)',
        ),
        '[first](https://relay.example/file?q=[) '
        '[second &#91;Q4&#93;](https://relay.example/file.pdf)',
      );
    });
    for (final prose in ['oops [ then ', 'oops [[ text ', 'oops [x] [ ']) {
      for (final prefix in ['!', '']) {
        test('$prefix recovers after unmatched prose $prose', () {
          expect(
            normalizeMarkdownLinks(
              '$prose$prefix[outer [inner] label](<$media>) '
              r'[report \[Q4\]](<https://relay.example/report.pdf>)',
            ),
            '$prose$prefix[outer &#91;inner&#93; label]($media) '
            '[report &#91;Q4&#93;](https://relay.example/report.pdf)',
          );
        });
      }
    }
    test('near-limit unmatched prose preserves every later attachment', () {
      const source = '[file](<https://relay.example/file.pdf>) ';
      const expected = '[file](https://relay.example/file.pdf) ';
      final count = (256 * 1024 - 2) ~/ source.length;
      final watch = Stopwatch()..start();
      final output = normalizeMarkdownLinks('[ ${source * count}');
      watch.stop();
      expect(output == '[ ${expected * count}', isTrue);
      expect(watch.elapsed, lessThan(const Duration(seconds: 2)));
    });
    test('escapes spaces and parentheses without double-encoding URLs', () {
      const raw = 'https://relay.example/media/photo (1).png?q=a%20b&v=2';
      const encoded =
          'https://relay.example/media/photo%20%281%29.png?q=a%20b&v=2';
      expect(normalizeMarkdownLinks('![photo](<$raw>)'), '![photo]($encoded)');
      expect(
        normalizeMarkdownLinks('![photo]($encoded)'),
        '![photo]($encoded)',
      );
      expect(
        normalizeMarkdownLinks('![photo](https://relay.example/a(b).png)'),
        '![photo](https://relay.example/a%28b%29.png)',
      );
    });
    test('normalizes prose surrounding multiple authored destinations', () {
      expect(
        normalizeMarkdownLinks('See <$media> ![one](<$media>) [two](<$media>)'),
        'See <$media> ![one]($media) [two]($media)',
      );
    });
    test('preserves attachment examples inside inline and fenced code', () {
      const example = '![photo](<$media>)';
      expect(normalizeMarkdownLinks('`$example`'), '`$example`');
      expect(
        normalizeMarkdownLinks('```md\n$example\n```'),
        '```md\n$example\n```',
      );
    });
  });

  group('code boundaries', () {
    const url = 'buzz://message?channel=channel-1&id=message-1';
    // Marks the prose runs mapMarkdownProse hands to its transform.
    String markProse(String prose) => prose.replaceAll(url, '<prose>');

    final cases = <({String name, String input, String expected})>[
      (
        name: 'single-backtick inline span',
        input: '`$url` then $url',
        expected: '`$url` then <prose>',
      ),
      (
        name: 'matching multi-backtick inline span',
        input: '``$url`` then $url',
        expected: '``$url`` then <prose>',
      ),
      (
        name: 'literal shorter backtick run in inline span',
        input: '``inside ` $url`` then $url',
        expected: '``inside ` $url`` then <prose>',
      ),
      (
        name: 'inline closer must have equal length',
        input: '``$url``` still code`` then $url',
        expected: '``$url``` still code`` then <prose>',
      ),
      (
        name: 'fence accepts a longer line-start closer',
        input: '```\n$url\n````\n$url',
        expected: '```\n$url\n````\n<prose>',
      ),
      (
        name: 'fence ignores an inline-looking backtick run',
        input: '```\n$url ``` still code\n```\n$url',
        expected: '```\n$url ``` still code\n```\n<prose>',
      ),
      (
        name: 'tilde fence indented three spaces',
        input: '   ~~~\n$url\n   ~~~\n$url',
        expected: '   ~~~\n$url\n   ~~~\n<prose>',
      ),
      (
        name: 'fence opens after deeper indentation',
        input: '    ~~~\n$url',
        expected: '    ~~~\n$url',
      ),
      (
        name: 'unclosed fence inside a quote',
        input: '> ```sh\n> $url',
        expected: '> ```sh\n> $url',
      ),
      (
        name: 'unclosed quoted fence ends with its quote',
        input: '> ```sh\n> $url\n\n$url',
        expected: '> ```sh\n> $url\n\n<prose>',
      ),
      (
        name: 'quoted fence ends at a lazy line',
        input: '> ```sh\n> $url\n$url',
        expected: '> ```sh\n> $url\n<prose>',
      ),
      (
        name: 'top-level fence does not close on a quoted fence',
        input: '```md\n> ```\n$url\n```\n$url',
        expected: '```md\n> ```\n$url\n```\n<prose>',
      ),
      (
        name: 'quoted fence closes inside the quote',
        input: '> ~~~\n> $url\n> ~~~\n$url',
        expected: '> ~~~\n> $url\n> ~~~\n<prose>',
      ),
      (
        name: 'fence closer must end its line',
        input: '~~~\n$url\n~~~ x\n~~~\n$url',
        expected: '~~~\n$url\n~~~ x\n~~~\n<prose>',
      ),
      (
        name: 'unclosed backticks remain prose',
        input: '$url then `$url',
        expected: '<prose> then `<prose>',
      ),
    ];

    for (final testCase in cases) {
      test(testCase.name, () {
        expect(mapMarkdownProse(testCase.input, markProse), testCase.expected);
      });
    }

    test('an escaped backtick does not open code', () {
      expect(mapMarkdownProse('\\`$url`', markProse), '\\`<prose>`');
    });

    test('near-limit fence-like runs on one line complete linearly', () {
      // Each run checks only its own indentation and the rest of its line up
      // to the next character, so a long line of runs is not rescanned.
      for (final input in [
        'x~~~' * (256 * 1024 ~/ 4),
        '~~~\n${' ~~~' * (256 * 1024 ~/ 4)}',
        '```\n${' ```' * (256 * 1024 ~/ 4)}',
        '>' * (256 * 1024 - 3) + '~~~',
        '> ~~~' * (256 * 1024 ~/ 5),
      ]) {
        final watch = Stopwatch()..start();
        final result = mapMarkdownProse(input, (prose) => prose);
        watch.stop();
        expect(result == input, isTrue);
        expect(watch.elapsed, lessThan(const Duration(seconds: 2)));
      }
    });
  });
}

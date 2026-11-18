import 'package:buzz/features/channels/message_content/link_normalizer.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  const url = 'buzz://message?channel=channel-1&id=message-1';

  test('normalizes supported bare and autolinked Buzz URLs', () {
    expect(
      normalizeBareLinks('See $url and <$url>'),
      'See [$url]($url) and [$url]($url)',
    );
  });

  test('leaves link reference definitions, but not lazy lines, as written', () {
    const site = 'https://example.com';
    for (final (markdown, expected) in [
      ('[a]: $site', '[a]: $site'),
      (
        'See [a]\n\n[a]: $site\n[b]: $site "B"',
        'See [a]\n\n[a]: $site\n[b]: $site "B"',
      ),
      ('look at this\n[note]: $site', 'look at this\n[note]: [$site]($site)'),
      (
        'look at `this`\n[note]: $site',
        'look at `this`\n[note]: [$site]($site)',
      ),
      ('\n[a]: $site', '\n[a]: $site'),
      ('`code` [x]: $site', '`code` [x]: [$site]($site)'),
      ('`x`\n\n[a]: $site', '`x`\n\n[a]: $site'),
      // A title holding code doesn't end the definition run.
      (
        '[a]: $site "`code`"\n[b]: $site\n\n[b]',
        '[a]: $site "`code`"\n[b]: $site\n\n[b]',
      ),
      (
        '[a]: $site "`code` [fake]: x"\n[b]: $site\n\n[b]',
        '[a]: $site "`code` [fake]: x"\n[b]: $site\n\n[b]',
      ),
      (
        'text\n[a]: $site\n[b]: $site',
        'text\n[a]: [$site]($site)\n[b]: [$site]($site)',
      ),
    ]) {
      expect(normalizeBareLinks(markdown), expected, reason: markdown);
    }
  });

  test('a long run of definitions takes linear time', () {
    // Rechecking every earlier definition for each one took seconds here.
    final body = [
      for (var i = 0; i < 4000; i++) '[d$i]: https://example.com/$i',
    ].join('\n');
    final timer = Stopwatch()..start();
    expect(normalizeBareLinks(body), body);
    expect(timer.elapsedMilliseconds, lessThan(500));
  });

  test('a long paragraph of unpaired backticks takes linear time', () {
    // Walking to the paragraph end for every opener took seconds at 64 KB.
    final body = 'a `\n` b' * (64 * 1024 ~/ 7);
    final timer = Stopwatch()..start();
    expect(mapMarkdownProse(body, (prose) => prose), body);
    expect(timer.elapsedMilliseconds, lessThan(500));
  });

  test('keeps punctuation and open Markdown delimiters outside links', () {
    expect(
      normalizeBareLinks('**open $url**. and **_${url}_**!'),
      '**open [$url]($url)**. and **_[$url]($url)_**!',
    );
  });

  test('preserves URL suffix characters without a matching opener', () {
    expect(
      normalizeBareLinks(
        'See $url'
        '_ and $url~~',
      ),
      'See [$url'
      '_]($url'
      '_) and [$url~~]($url~~)',
    );
  });

  group('attachment destinations', () {
    const media = 'https://relay.example/media/image.png';
    for (final prefix in ['!', '']) {
      test('$prefix preserves angle destinations and optional titles', () {
        for (final destination in ['<$media>', media, '<$media> "Title"']) {
          expect(
            normalizeBareLinks('$prefix[label]($destination)'),
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
            normalizeBareLinks('$prefix[label](<$media> $title)'),
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
        final result = normalizeBareLinks(input);
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
            normalizeBareLinks('$prefix[$label](<$media>)'),
            '$prefix[$rendered]($media)',
          );
        });
      }
    }
    test('URL and title brackets do not capture a later attachment label', () {
      expect(
        normalizeBareLinks(
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
            normalizeBareLinks(
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
      final output = normalizeBareLinks('[ ${source * count}');
      watch.stop();
      expect(output == '[ ${expected * count}', isTrue);
      expect(watch.elapsed, lessThan(const Duration(seconds: 2)));
    });
    test('escapes spaces and parentheses without double-encoding URLs', () {
      const raw = 'https://relay.example/media/photo (1).png?q=a%20b&v=2';
      const encoded =
          'https://relay.example/media/photo%20%281%29.png?q=a%20b&v=2';
      expect(normalizeBareLinks('![photo](<$raw>)'), '![photo]($encoded)');
      expect(normalizeBareLinks('![photo]($encoded)'), '![photo]($encoded)');
      expect(
        normalizeBareLinks('![photo](https://relay.example/a(b).png)'),
        '![photo](https://relay.example/a%28b%29.png)',
      );
    });
    test('normalizes prose surrounding multiple authored destinations', () {
      expect(
        normalizeBareLinks('See <$media> ![one](<$media>) [two](<$media>)'),
        'See [$media]($media) ![one]($media) [two]($media)',
      );
    });
    test('preserves attachment examples inside inline and fenced code', () {
      const example = '![photo](<$media>)';
      expect(normalizeBareLinks('`$example`'), '`$example`');
      expect(
        normalizeBareLinks('```md\n$example\n```'),
        '```md\n$example\n```',
      );
    });
  });

  group('code boundaries', () {
    final cases = <({String name, String input, String expected})>[
      (
        name: 'single-backtick inline span',
        input: '`$url` then $url',
        expected: '`$url` then [$url]($url)',
      ),
      (
        name: 'matching multi-backtick inline span',
        input: '``$url`` then $url',
        expected: '``$url`` then [$url]($url)',
      ),
      (
        name: 'literal shorter backtick run in inline span',
        input: '``inside ` $url`` then $url',
        expected: '``inside ` $url`` then [$url]($url)',
      ),
      (
        name: 'inline closer must have equal length',
        input: '``$url``` still code`` then $url',
        expected: '``$url``` still code`` then [$url]($url)',
      ),
      (
        name: 'fence accepts a longer line-start closer',
        input: '```\n$url\n````\n$url',
        expected: '```\n$url\n````\n[$url]($url)',
      ),
      (
        name: 'fence ignores an inline-looking backtick run',
        input: '```\n$url ``` still code\n```\n$url',
        expected: '```\n$url ``` still code\n```\n[$url]($url)',
      ),
      (
        name: 'tilde fence indented three spaces',
        input: '   ~~~\n$url\n   ~~~\n$url',
        expected: '   ~~~\n$url\n   ~~~\n[$url]($url)',
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
        expected: '> ```sh\n> $url\n\n[$url]($url)',
      ),
      (
        name: 'quoted fence ends at a lazy line',
        input: '> ```sh\n> $url\n$url',
        expected: '> ```sh\n> $url\n[$url]($url)',
      ),
      (
        name: 'top-level fence does not close on a quoted fence',
        input: '```md\n> ```\n$url\n```\n$url',
        expected: '```md\n> ```\n$url\n```\n[$url]($url)',
      ),
      (
        name: 'quoted fence closes inside the quote',
        input: '> ~~~\n> $url\n> ~~~\n$url',
        expected: '> ~~~\n> $url\n> ~~~\n[$url]($url)',
      ),
      (
        name: 'fence closer must end its line',
        input: '~~~\n$url\n~~~ x\n~~~\n$url',
        expected: '~~~\n$url\n~~~ x\n~~~\n[$url]($url)',
      ),
      (
        name: 'unmatched backtick does not pair across a block quote',
        input: 'plain `\n> $url `code`',
        expected: 'plain `\n> [$url]($url) `code`',
      ),
      (
        name: 'unmatched backtick does not pair into a nested quote',
        input: '> plain `\n> > $url `code`',
        expected: '> plain `\n> > [$url]($url) `code`',
      ),
      (
        name: 'quoted code span continues on the next quoted line',
        input: '> `$url\n> more` $url',
        expected: '> `$url\n> more` [$url]($url)',
      ),
      (
        name: 'unclosed backticks remain prose',
        input: '$url then `$url',
        expected: '[$url]($url) then `[$url]($url)',
      ),
    ];

    for (final testCase in cases) {
      test(testCase.name, () {
        expect(normalizeBareLinks(testCase.input), testCase.expected);
      });
    }

    test('bare links leave a trailing backslash outside the destination', () {
      expect(
        normalizeBareLinks(r'see https://example.com/a\'),
        r'see [https://example.com/a](https://example.com/a)\',
      );
      expect(
        normalizeBareLinks(r'*https://example.com/a\**'),
        r'*[https://example.com/a](https://example.com/a)\**',
      );
    });

    test('a literal backslash before a bare link stays literal', () {
      expect(
        normalizeBareLinks(r'see \https://example.com/a'),
        r'see \\[https://example.com/a](https://example.com/a)',
      );
      expect(
        normalizeBareLinks(r'see \\https://example.com/a'),
        r'see \\[https://example.com/a](https://example.com/a)',
      );
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
        final result = normalizeBareLinks(input);
        watch.stop();
        expect(result == input, isTrue);
        expect(watch.elapsed, lessThan(const Duration(seconds: 2)));
      }
    });
  });

  test(
    'preserves HTTP(S) destinations while retaining bare-link rendering',
    () {
      const httpUrl = 'https://example.com/search?q=why?';
      expect(
        normalizeBareLinks('See $httpUrl and <$httpUrl>'),
        'See [$httpUrl]($httpUrl) and [$httpUrl]($httpUrl)',
      );
    },
  );
  test('normalizes every bare Buzz entity permalink family', () {
    final owner = 'ab' * 32;
    final id = 'cd' * 32;
    final links = [
      'buzz://repo?owner=$owner&d=buzz',
      'buzz://pr?id=$id&owner=$owner&d=buzz',
      'buzz://issue?id=$id&owner=$owner&d=buzz',
    ];
    for (final link in links) {
      expect(normalizeBareLinks('$link.'), '[$link]($link).');
    }
  });
}

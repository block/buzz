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
}

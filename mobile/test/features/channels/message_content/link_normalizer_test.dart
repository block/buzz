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

  test('preserves angle-bracket Buzz Markdown destinations', () {
    const channelUrl = 'buzz://channel/550e8400-e29b-41d4-a716-446655440000';
    const messageUrl = 'buzz://message?channel=channel-1&id=message-1';
    expect(
      normalizeBareLinks(
        '[channel](<$channelUrl>) [message](<$messageUrl>) '
        '<$channelUrl> <$messageUrl>',
      ),
      '[channel]($channelUrl) [message]($messageUrl) '
      '[$channelUrl]($channelUrl) [$messageUrl]($messageUrl)',
    );
  });

  test('keeps prose closing delimiters outside bare Buzz URLs', () {
    const channelUrl = 'buzz://channel/550e8400-e29b-41d4-a716-446655440000';
    const messageUrl = 'buzz://message?channel=channel-1&id=message-1';
    final cases = <(String, String)>[
      ('"', '"'),
      ("'", "'"),
      ('“', '”'),
      ('‘', '’'),
      ('«', '»'),
      ('‹', '›'),
      ('《', '》'),
      ('〈', '〉'),
      ('「', '」'),
      ('『', '』'),
      ('【', '】'),
      ('〔', '〕'),
      ('〖', '〗'),
      ('〘', '〙'),
      ('〚', '〛'),
    ];

    for (final (opening, closing) in cases) {
      expect(
        normalizeBareLinks('$opening$channelUrl$closing'),
        '$opening[$channelUrl]($channelUrl)$closing',
      );
      expect(
        normalizeBareLinks('$opening$messageUrl$closing.'),
        '$opening[$messageUrl]($messageUrl)$closing.',
      );
    }
    expect(
      normalizeBareLinks(
        'See $channelUrl。 Then $messageUrl！ Also $channelUrl．',
      ),
      'See [$channelUrl]($channelUrl)。 Then [$messageUrl]($messageUrl)！ '
      'Also [$channelUrl]($channelUrl)．',
    );
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

  test('does not relink URLs owned by existing Markdown or attributes', () {
    const httpUrl = 'https://example.com/file.png';
    expect(
      normalizeBareLinks(
        '[label]($httpUrl) ![image]($httpUrl) '
        'imeta=url=$httpUrl escaped \\$httpUrl',
      ),
      '[label]($httpUrl) ![image]($httpUrl) '
      'imeta=url=$httpUrl escaped \\$httpUrl',
    );
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
}

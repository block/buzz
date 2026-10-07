import 'package:buzz/features/channels/message_content/details_blocks.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('splits text and a section whose closing marker follows a list', () {
    final segments = splitDetailsBlocks(
      'Board\n:::details Waiting (2)\n- one\n- two\n:::\nAfter',
    );
    expect(segments, hasLength(3));
    expect((segments[0] as DetailsText).text, 'Board');
    final block = segments[1] as DetailsBlock;
    expect(block.title, 'Waiting (2)');
    expect(block.body, '- one\n- two');
    expect(block.key, '0:Waiting (2)');
    expect((segments[2] as DetailsText).text, 'After');
  });

  test('numbers repeated titles so keys stay distinct', () {
    final keys = splitDetailsBlocks(
      ':::details A\none\n:::\n:::details A\ntwo\n:::',
    ).whereType<DetailsBlock>().map((block) => block.key);
    expect(keys, ['0:A', '1:A']);
  });

  test('keeps nested sections inside the top-level body', () {
    final segments = splitDetailsBlocks(
      ':::details Outer\n:::details Inner\nx\n:::\n:::',
    );
    final outer = segments.single as DetailsBlock;
    expect(outer.body, ':::details Inner\nx\n:::');
    final inner = splitDetailsBlocks(outer.body).single as DetailsBlock;
    expect(inner.title, 'Inner');
  });

  test('an unclosed opener stays text without hiding later sections', () {
    final segments = splitDetailsBlocks(
      ':::details Never closed\n:::details Closed\nx\n:::',
    );
    expect((segments[0] as DetailsText).text, ':::details Never closed');
    expect((segments[1] as DetailsBlock).title, 'Closed');
  });

  test('ignores markers inside fenced or indented code, or quoted', () {
    for (final content in [
      '```\n:::details Not a section\n:::\n```',
      'Code:\n\n    :::details Not a section\n    x\n    :::',
      '- item\n\n    ```\n    :::details Not a section\n    :::\n    ```',
      '- item\n\n    ```\n  :::details Not a section\n  secret\n  :::\n    ```',
      '> :::details Quoted\n> x\n> :::',
    ]) {
      final segments = splitDetailsBlocks(content);
      expect(segments.single, isA<DetailsText>(), reason: content);
      expect((segments.single as DetailsText).text, content);
    }
  });

  test('accepts markers a composer paste indented into a list item', () {
    // What the composer sends after pasting a section that ends in a list:
    // the closer becomes a continuation line of the last item.
    final segments = splitDetailsBlocks(
      ':::details **Fleet working** (2)\n\n- model routing\n'
      '- OpenClaw upgrade\n  :::\n\n- x\n  :::details Next\nbody\n:::',
    );
    expect(segments, hasLength(3));
    final first = segments[0] as DetailsBlock;
    expect(first.title, 'Fleet working (2)');
    expect(first.body, '\n- model routing\n- OpenClaw upgrade');
    expect((segments[1] as DetailsText).text, '\n- x');
    expect((segments[2] as DetailsBlock).title, 'Next');
  });

  test('accepts no-break spaces the composer sent around a bold title', () {
    // Bytes a user's composer sent: U+00A0 after `:::details` and
    // after the closing `**`, items as escaped-dash paragraphs.
    final segments = splitDetailsBlocks(
      ':::details **Fleet working** (2)\n\n\\- model routing\n\n'
      '\\- OpenClaw upgrade\n\n:::',
    );
    final block = segments.single as DetailsBlock;
    expect(block.title, 'Fleet working (2)');
    expect(block.titleMarkdown, '**Fleet working** (2)');
    expect(block.body, '\n\\- model routing\n\n\\- OpenClaw upgrade\n');
    expect(normalizeMarkerLine(':::details ## Title'), ':::details ## Title');
    expect(normalizeMarkerLine('**:::details X** '), ':::details **X**');
    expect(normalizeMarkerLine('::: '), ':::');
    expect(normalizeMarkerLine(':::details '), isNull);
  });

  test('flattens sections for previews', () {
    expect(
      flattenDetailsBlocks('Board\n:::details Private (1)\n- item\n:::'),
      'Board\nPrivate (1)\n- item',
    );
    expect(flattenDetailsBlocks('plain'), 'plain');
  });

  test('markers past the depth limit stay text', () {
    const depth = maxDetailsDepth + 1;
    final content = [
      for (var i = 0; i < depth; i++) ':::details L$i',
      'x',
      for (var i = 0; i < depth; i++) ':::',
    ].join('\n');
    var segments = splitDetailsBlocks(content);
    var levels = 0;
    while (segments.whereType<DetailsBlock>().isNotEmpty) {
      final block = segments.whereType<DetailsBlock>().single;
      levels++;
      segments = splitDetailsBlocks(block.body, depth: levels);
    }
    expect(levels, maxDetailsDepth);
    expect(
      (segments.single as DetailsText).text,
      contains(':::details L$maxDetailsDepth'),
    );
  });

  test('stays linear on hostile nesting near the message size limit', () {
    final content = '${':::details x\n' * 3000}${':::\n' * 3000}';
    expect(content.length, lessThan(64 * 1024));
    final watch = Stopwatch()..start();
    final segments = splitDetailsBlocks(content);
    final flat = flattenDetailsBlocks(content);
    expect(watch.elapsed, lessThan(const Duration(seconds: 2)));
    expect(segments.whereType<DetailsBlock>(), hasLength(1));
    expect(flat, isNotEmpty);
  });

  test('an info-string line does not close a fence', () {
    const content = '```js\n``` trailing\n:::details Inside code\nx\n:::\n```';
    expect(splitDetailsBlocks(content).single, isA<DetailsText>());
  });

  test('recognises markers with CRLF line endings', () {
    final block = splitDetailsBlocks(
      'Board\r\n:::details A\r\nx\r\n:::\r\nAfter',
    ).whereType<DetailsBlock>().single;
    expect(block.title, 'A');
    expect(block.key, '0:A');
  });

  test('titles are plain text', () {
    final block = splitDetailsBlocks(
      ':::details [docs](https://example.com) **now**\nx\n:::',
    ).whereType<DetailsBlock>().single;
    expect(block.title, 'docs now');
  });

  test('title text matches desktop plain text', () {
    expect(plainDetailsTitle('_italic_ snake_case'), 'italic snake_case');
    expect(plainDetailsTitle('Icon ![lock](https://x/y.png)'), 'Icon lock');
    expect(plainDetailsTitle('<https://example.com>'), 'https://example.com');
    expect(plainDetailsTitle('[](https://example.com)'), detailsFallbackTitle);
  });

  test('accepts marker lines the composer wrapped in bold', () {
    final segments = splitDetailsBlocks(
      '**:::details Bold (2)**\n**- one**\n**:::**\nAfter',
    );
    final block = segments.whereType<DetailsBlock>().single;
    expect(block.title, 'Bold (2)');
    expect(block.titleMarkdown, '**Bold (2)**');
    expect(block.body, '**- one**');
    expect((segments.last as DetailsText).text, 'After');
    expect(normalizeMarkerLine('**:::details ## X**'), ':::details ## **X**');
    expect(normalizeMarkerLine('_:::_'), ':::');
    expect(normalizeMarkerLine('**text**'), isNull);
  });

  test('a heading title carries its level and keeps formatting', () {
    final block = splitDetailsBlocks(
      ':::details ## [docs](https://x) **now**\nx\n:::',
    ).whereType<DetailsBlock>().single;
    expect(block.level, 2);
    expect(block.title, 'docs now');
    expect(block.titleMarkdown, 'docs **now**');
    expect(block.key, '0:docs now');
  });

  test('titles match desktop for images and parenthesised links', () {
    final image = splitDetailsBlocks(
      ':::details ![lock](https://x/i.png)\nx\n:::',
    ).whereType<DetailsBlock>().single;
    expect(image.title, 'lock');
    expect(plainDetailsTitle('[docs](https://x/a_(b))'), 'docs');
  });

  test('title sanitizing stays linear on hostile 64 KiB titles', () {
    for (final unit in ['[', '![', '*', '#', ' ', '_', '[a](']) {
      final title = unit * (64 * 1024 ~/ unit.length);
      final watch = Stopwatch()..start();
      splitDetailsBlocks(':::details x$title\nbody\n:::');
      expect(
        watch.elapsed,
        lessThan(const Duration(milliseconds: 500)),
        reason: 'unit "$unit"',
      );
    }
  });

  test('long constructs in titles still flatten', () {
    final url = 'https://media.example/${'a' * 400}?sig=${'b' * 100}';
    final block = splitDetailsBlocks(
      ':::details [docs]($url) **${'c' * 200}**\nx\n:::',
    ).whereType<DetailsBlock>().single;
    expect(block.title, 'docs ${'c' * 200}');
    expect(block.titleMarkdown, 'docs **${'c' * 200}**');
    expect(block.key, '0:docs ${'c' * 200}');
    expect(
      splitDetailsBlocks(
        ':::details ![lock]($url)\nx\n:::',
      ).whereType<DetailsBlock>().single.title,
      'lock',
    );
  });

  test('titles over the limit are cut, never inside a surrogate pair', () {
    final long = splitDetailsBlocks(
      ':::details ${'a' * (maxDetailsTitleLength + 50)}\nx\n:::',
    ).whereType<DetailsBlock>().single;
    expect(long.title.length, maxDetailsTitleLength);
    final emoji = splitDetailsBlocks(
      ':::details ${'a' * (maxDetailsTitleLength - 1)}😀 tail\nx\n:::',
    ).whereType<DetailsBlock>().single;
    expect(emoji.title, 'a' * (maxDetailsTitleLength - 1));
  });

  test('a message full of hostile titles stays bounded', () {
    final section = ':::details ${'[' * maxDetailsTitleLength}\nx\n:::\n';
    final content = section * (64 * 1024 ~/ section.length);
    final watch = Stopwatch()..start();
    splitDetailsBlocks(content);
    expect(watch.elapsed, lessThan(const Duration(seconds: 1)));
  });
}

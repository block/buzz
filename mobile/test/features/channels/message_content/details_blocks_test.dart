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

  test('ignores markers inside fenced code, indented, or quoted', () {
    for (final content in [
      '```\n:::details Not a section\n:::\n```',
      '  :::details Indented\nx\n  :::',
      '> :::details Quoted\n> x\n> :::',
    ]) {
      final segments = splitDetailsBlocks(content);
      expect(segments.single, isA<DetailsText>(), reason: content);
      expect((segments.single as DetailsText).text, content);
    }
  });

  test('flattens sections for previews', () {
    expect(
      flattenDetailsBlocks('Board\n:::details Private (1)\n- item\n:::'),
      'Board\nPrivate (1)\n- item',
    );
    expect(flattenDetailsBlocks('plain'), 'plain');
  });
}

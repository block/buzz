import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/message_content/preview_source.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show RenderRepaintBoundary;
import 'package:flutter_test/flutter_test.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

Widget _testable(Widget child) => ProviderScope(
  child: MaterialApp(
    theme: AppTheme.light(),
    builder: (context, child) => AppMarkdownTheme(child: child!),
    home: Scaffold(
      body: Align(
        alignment: Alignment.topLeft,
        child: SizedBox(width: 320, child: child),
      ),
    ),
  ),
);

String _paragraphs(int count) => [
  for (var i = 0; i < count; i++) 'Paragraph $i with **bold** and `code`.',
].join('\n\n');

final _code = [
  '```dart',
  for (var i = 0; i < 30; i++) 'final x$i = $i;',
  '```',
];

void main() {
  // Each case is rendered with and without the cut; the pixels must match.
  final cases = <({String name, String source, int maxLines, bool cut})>[
    (name: 'paragraphs', source: _paragraphs(12), maxLines: 2, cut: true),
    (name: 'one-line preview', source: _paragraphs(12), maxLines: 1, cut: true),
    (
      name: 'soft-wrapped lines',
      source: [for (var i = 0; i < 20; i++) 'line $i ' * 6].join('\n'),
      maxLines: 2,
      cut: true,
    ),
    (
      name: 'list items',
      source: [
        'Status:',
        for (var i = 0; i < 20; i++) '- item $i with [a link](https://x.y/$i)',
      ].join('\n'),
      maxLines: 2,
      cut: true,
    ),
    (
      name: 'code in the tail',
      source: ['Intro.', 'More.', 'Then:', ..._code, 'After.'].join('\n'),
      maxLines: 2,
      cut: true,
    ),
    (
      name: 'code first',
      source: [..._code, 'One.', 'Two.', 'Three.', 'Four.'].join('\n'),
      maxLines: 2,
      cut: true,
    ),
    (
      name: 'table first',
      source: [
        '| a | b |',
        '|---|---|',
        for (var i = 0; i < 10; i++) '| $i | ${i * 2} |',
        '',
        _paragraphs(6),
      ].join('\n'),
      maxLines: 2,
      cut: true,
    ),
    (
      name: 'quote first',
      source: ['> quoted', '> still quoted', '', _paragraphs(6)].join('\n'),
      maxLines: 2,
      cut: true,
    ),
    (
      name: 'heading',
      source: ['## Results', _paragraphs(6)].join('\n'),
      maxLines: 2,
      cut: true,
    ),
    (
      name: 'backticks inside a fence',
      source: [
        '```dart',
        "final fence = '```';",
        'final x = 1;',
        '```',
        'One.',
        'Two.',
        'Three.',
      ].join('\n'),
      maxLines: 2,
      cut: true,
    ),
    (
      name: 'inline code beside bold',
      source: ['`a` **b** `c`', 'two', 'three', 'four'].join('\n'),
      maxLines: 1,
      cut: true,
    ),
    (name: 'short', source: 'One.\n\nTwo.\n\nThree.', maxLines: 2, cut: false),
    (
      name: 'bold closing in the tail',
      source: ['**Opened', 'two', 'three', 'four', 'closed**'].join('\n'),
      maxLines: 2,
      cut: false,
    ),
    (
      name: 'code closing in the tail',
      source: ['Intro `opened', 'two', 'three', 'four', 'closed`'].join('\n'),
      maxLines: 2,
      cut: false,
    ),
    (
      name: 'link closing in the tail',
      source: ['[opened', 'two', 'three', 'four', 'x](https://x.y)'].join('\n'),
      maxLines: 2,
      cut: false,
    ),
    (
      name: 'bold marker inside inline code',
      source: ['`**` **opened', 'two', 'three', 'closed**'].join('\n'),
      maxLines: 1,
      cut: false,
    ),
    (
      name: 'backticks inside a line',
      source: ['use ```x``` here', 'two', 'three', 'four'].join('\n'),
      maxLines: 1,
      cut: false,
    ),
    (
      name: 'double backticks',
      source: ['a ``b`` c', 'two', 'three', 'four'].join('\n'),
      maxLines: 1,
      cut: false,
    ),
    (
      name: 'stray closer before an opener',
      source: [
        '</u> stray',
        '<u>opened',
        'still open',
        'more',
        'closed</u>',
      ].join('\n'),
      maxLines: 2,
      cut: false,
    ),
    (
      name: 'unclosed fence',
      source: ['```', for (var i = 0; i < 10; i++) 'line $i'].join('\n'),
      maxLines: 2,
      cut: false,
    ),
  ];

  Future<(Size, Uint8List)> render(
    WidgetTester tester,
    String text,
    int maxLines,
  ) async {
    final key = GlobalKey();
    await tester.pumpWidget(
      _testable(
        RepaintBoundary(
          key: key,
          child: GptMarkdown(text, maxLines: maxLines),
        ),
      ),
    );
    await tester.pumpAndSettle();
    return (await tester.runAsync(() async {
      final boundary =
          key.currentContext!.findRenderObject()! as RenderRepaintBoundary;
      final image = await boundary.toImage();
      final bytes = (await image.toByteData(
        format: ui.ImageByteFormat.rawRgba,
      ))!;
      final size = Size(image.width.toDouble(), image.height.toDouble());
      image.dispose();
      return (size, bytes.buffer.asUint8List());
    }))!;
  }

  for (final c in cases) {
    testWidgets('a preview of ${c.name} renders the same after the cut', (
      tester,
    ) async {
      final kept = markdownPreviewSource(c.source, c.maxLines);
      expect(kept.length < c.source.length, c.cut);
      final (fullSize, full) = await render(tester, c.source, c.maxLines);
      final (keptSize, cut) = await render(tester, kept, c.maxLines);
      expect(keptSize, fullSize);
      expect(cut, full);
    });
  }

  test('a crafted run of backtick pairs is scanned in linear time', () {
    // Quadratic matching took about 17 s on this input; linear takes ~1 ms.
    final source = ['[x](${'``x' * 20000})', 'one', 'two', 'three'].join('\n');
    final watch = Stopwatch()..start();
    expect(markdownPreviewSource(source, 2), source);
    expect(watch.elapsed, lessThan(const Duration(seconds: 1)));
  });

  testWidgets('a capped message leaves out the tail it cannot show', (
    tester,
  ) async {
    final content = '${_paragraphs(8)}\n\nTAILMARK';
    await tester.pumpWidget(
      _testable(MessageContent(content: content, maxLines: 2)),
    );
    expect(find.textContaining('TAILMARK', findRichText: true), findsNothing);

    await tester.pumpWidget(_testable(MessageContent(content: content)));
    expect(find.textContaining('TAILMARK', findRichText: true), findsWidgets);
  });
}

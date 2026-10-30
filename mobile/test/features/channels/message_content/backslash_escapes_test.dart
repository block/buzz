import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/message_content/backslash_escapes.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

Widget _host(
  String content, {
  Map<String, String> mentions = const {},
  void Function(String)? onMentionTap,
}) => ProviderScope(
  child: MaterialApp(
    theme: AppTheme.light(),
    home: AppMarkdownTheme(
      child: Scaffold(
        body: MessageContent(
          content: content,
          mentionNames: mentions,
          channelNames: const {'sample': 'channel-one'},
          onMentionTap: onMentionTap,
        ),
      ),
    ),
  ),
);

List<RichText> _paragraphs(WidgetTester tester) => tester
    .widgetList<RichText>(find.byWidgetPredicate((w) => w is RichText))
    .toList();

String _plain(WidgetTester tester) => _paragraphs(tester)
    .map(
      (rt) =>
          rt.text.toPlainText().replaceAll(RegExp('[\uFFFC\uE100-\uF8FF]'), ''),
    )
    .where((text) => text.isNotEmpty)
    .join('\n')
    .trimRight();

bool _hasRecognizerOrLink(WidgetTester tester) =>
    find.byType(GestureDetector).evaluate().any((e) {
      final widget = e.widget as GestureDetector;
      return widget.onTap != null &&
          find
              .descendant(
                of: find.byWidget(widget),
                matching: find.byType(RichText),
              )
              .evaluate()
              .isNotEmpty;
    });

final _punctuation = String.fromCharCodes([
  for (var code = 0x21; code <= 0x7E; code++)
    if (!RegExp(r'[0-9A-Za-z]').hasMatch(String.fromCharCode(code))) code,
]);
final _escapedPunctuation = _punctuation.split('').map((c) => '\\$c').join();

void main() {
  final cases = <(String name, String source, String shown)>[
    ('shrug', r'¯\\\_(ツ)\_/¯', r'¯\_(ツ)_/¯'),
    ('escaped emphasis', r'\*not italic\*', '*not italic*'),
    ('escaped underscores', r'\_not italic\_', '_not italic_'),
    ('escaped heading marker', r'\# not heading', '# not heading'),
    ('escaped backslash', r'a\\b', r'a\b'),
    ('backslash before a letter', r'C:\path', r'C:\path'),
    ('escaped link brackets', r'\[x\](y)', '[x](y)'),
    (
      'escaped brackets around a link target',
      r'\[x](https://e.com)',
      '[x](https://e.com)',
    ),
    ('escaped backticks', r'\`not code\`', '`not code`'),
    ('escaped delimiter inside emphasis', r'**a\*b**', 'a*b'),
    ('escaped bracket in a link label', r'[a\]b](https://e.com)', 'a]b'),
    ('every ASCII punctuation mark', _escapedPunctuation, _punctuation),
    ('backslash before a non-ASCII character', r'\é', r'\é'),
    ('backslash before a space', r'a\ b', r'a\ b'),
    ('hard break', 'one\\\ntwo', 'one\ntwo'),
    ('backslash at the end of the message', r'end\', r'end\'),
    ('backslash before a blank line', 'one\\\n\ntwo', 'one\\\n\ntwo'),
    ('backslash in a bare URL', r'https://e.com/a\_b', r'https://e.com/a\_b'),
    (
      'backslash in an autolink',
      r'<https://e.com/a\_b>',
      r'https://e.com/a\_b',
    ),
    ('inline code', r'`a\\b \*x\* \_`', r'a\\b \*x\* \_'),
    (
      'code fence',
      '```\na\\\\b \\*x\\* \\# \\[x\\](y)\n```',
      r'a\\b \*x\* \# \[x\](y)',
    ),
  ];

  for (final (name, source, shown) in cases) {
    testWidgets('renders $name as authored in CommonMark', (tester) async {
      await tester.pumpWidget(_host(source));
      expect(_plain(tester), shown);
      expect(RegExp('[\uE021-\uE07E]').hasMatch(_plain(tester)), isFalse);
    });
  }

  testWidgets('escaped markup is not styled or linked', (tester) async {
    await tester.pumpWidget(_host(r'\*plain\* \[x\](https://e.com) \# h'));
    expect(_hasRecognizerOrLink(tester), isFalse);
    final span = _paragraphs(tester).single.text as TextSpan;
    final styles = <TextStyle?>{};
    span.visitChildren((s) {
      if (s is TextSpan && s.text != null) styles.add(s.style);
      return true;
    });
    expect(styles.any((s) => s?.fontWeight == FontWeight.bold), isFalse);
    expect(styles.any((s) => s?.fontStyle == FontStyle.italic), isFalse);
  });

  testWidgets('mentions, channels and formatting render beside escapes', (
    tester,
  ) async {
    String? tapped;
    await tester.pumpWidget(
      _host(
        r'**bold** \*lit\* @Alice in #sample',
        mentions: {'alice-key': 'Alice'},
        onMentionTap: (id) => tapped = id,
      ),
    );
    expect(find.text('Alice'), findsOneWidget);
    expect(find.text('sample'), findsOneWidget);
    await tester.tap(find.text('Alice'));
    expect(tapped, 'alice-key');
    expect(_plain(tester), contains('bold *lit*'));
  });

  group('shieldBackslashEscapes', () {
    test('leaves content without a backslash untouched', () {
      const source = '**a** `b` @c';
      expect(identical(shieldBackslashEscapes(source), source), isTrue);
    });

    test('round-trips every escape through unshield', () {
      expect(
        unshieldBackslashEscapes(shieldBackslashEscapes(r'a\*b\\c\`d')),
        r'a*b\c`d',
      );
    });

    test('an escaped backtick does not open code that hides later escapes', () {
      expect(
        unshieldBackslashEscapes(shieldBackslashEscapes(r'\` \* `x`')),
        '` * `x`',
      );
    });
  });
}

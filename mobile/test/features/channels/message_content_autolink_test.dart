import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

final _url =
    'buzz://message?channel=580ca78b-9dae-46f3-8854-bd671853ba32'
    '&id=${'ab' * 32}';

Future<void> _pump(WidgetTester tester, String content) {
  return tester.pumpWidget(
    ProviderScope(
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: SingleChildScrollView(child: MessageContent(content: content)),
        ),
      ),
    ),
  );
}

/// Every link MessageContent drew: Buzz permalinks with a canonical label
/// render as chips, everything else as a [LinkTextSpan].
List<String> _links(WidgetTester tester) {
  final links = <String>[];
  // Not `visitChildren`: it only visits spans that carry their own text, and
  // a link wrapping its parsed label carries none.
  void walk(InlineSpan span) {
    if (span is LinkTextSpan) links.add(span.url);
    if (span is TextSpan) span.children?.forEach(walk);
  }

  for (final rich in tester.widgetList<RichText>(
    find.byWidgetPredicate((widget) => widget is RichText),
  )) {
    walk(rich.text);
  }
  for (final element
      in find
          .byWidgetPredicate(
            (widget) =>
                widget.key is ValueKey<String> &&
                (widget.key! as ValueKey<String>).value.startsWith(
                  'buzz-link-chip:',
                ),
          )
          .evaluate()) {
    final key = element.widget.key! as ValueKey<String>;
    links.add(key.value.substring('buzz-link-chip:'.length));
  }
  return links;
}

String _plainText(WidgetTester tester) => tester
    .widgetList<RichText>(
      find.byWidgetPredicate((widget) => widget is RichText),
    )
    .map((widget) => widget.text.toPlainText())
    .join('\n');

void main() {
  testWidgets('links bare and angle-bracketed Buzz URLs', (tester) async {
    await _pump(tester, 'See $_url and <$_url>');

    expect(_links(tester), [_url, _url]);
  });

  testWidgets('keeps punctuation and Markdown delimiters out of links', (
    tester,
  ) async {
    await _pump(tester, '**open $_url**. and **_${_url}_**!');

    expect(_links(tester), [_url, _url]);
    expect(_plainText(tester), isNot(contains('*')));
    expect(_plainText(tester), isNot(contains('_!')));
  });

  group('code boundaries', () {
    final cases = <({String name, String input})>[
      (name: 'single-backtick inline span', input: '`$_url` then $_url'),
      (
        name: 'matching multi-backtick inline span',
        input: '``$_url`` then $_url',
      ),
      (
        name: 'literal shorter backtick run in inline span',
        input: '``inside ` $_url`` then $_url',
      ),
      (
        name: 'fence accepts a longer line-start closer',
        input: '```\n$_url\n````\n$_url',
      ),
      (
        name: 'fence ignores an inline-looking backtick run',
        input: '```\n$_url ``` still code\n```\n$_url',
      ),
    ];

    for (final testCase in cases) {
      testWidgets('${testCase.name} stays code', (tester) async {
        await _pump(tester, testCase.input);

        expect(_links(tester), [_url]);
        expect(_plainText(tester), contains(_url));
      });
    }
  });

  testWidgets('links bare HTTP(S) URLs without trailing punctuation', (
    tester,
  ) async {
    const httpUrl = 'https://example.com/search?q=why';
    await _pump(tester, 'See $httpUrl? and <$httpUrl?>');

    expect(_links(tester), unorderedEquals([httpUrl, '$httpUrl?']));
  });

  testWidgets('links every bare Buzz entity permalink family', (tester) async {
    final owner = 'ab' * 32;
    final id = 'cd' * 32;
    final links = [
      'buzz://repo?owner=$owner&d=buzz',
      'buzz://pr?id=$id&owner=$owner&d=buzz',
      'buzz://issue?id=$id&owner=$owner&d=buzz',
    ];
    await _pump(tester, links.map((link) => '$link.').join('\n\n'));

    expect(_links(tester), unorderedEquals(links));
  });
}

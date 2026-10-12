part of 'message_content_test.dart';

void bubbleContentTests() {
  final bubbles = find.byWidgetPredicate(
    (widget) =>
        widget.key is ValueKey<String> &&
        (widget.key! as ValueKey<String>).value.startsWith(
          'message-text-bubble-',
        ),
  );
  testWidgets('only a group-ending bubble paints a left tail', (tester) async {
    for (final tail in [false, true]) {
      await tester.pumpWidget(
        _testable(
          MessageContent(content: 'Hello', bubble: true, bubbleTail: tail),
        ),
      );
      await tester.pumpAndSettle();
      final decoration =
          tester.widget<Container>(bubbles).decoration! as ShapeDecoration;
      final bounds = decoration.shape
          .getOuterPath(Offset.zero & tester.getSize(bubbles))
          .getBounds();
      expect(bounds.left, tail ? lessThan(0) : equals(0));
    }
  });

  testWidgets('photo gallery stays outside its caption bubble', (tester) async {
    await tester.pumpWidget(
      _testable(
        const MessageContent(
          content:
              'Photos\n![one](https://example.com/one.png)\n![two](https://example.com/two.png)',
          bubble: true,
        ),
      ),
    );
    await tester.pumpAndSettle();
    final gallery = find.byKey(const ValueKey('message-media-carousel'));
    expect(gallery, findsOneWidget);
    expect(bubbles, findsOneWidget);
    expect(find.descendant(of: bubbles, matching: gallery), findsNothing);
    expect(
      tester.getTopLeft(gallery).dy,
      greaterThanOrEqualTo(tester.getBottomLeft(bubbles).dy),
    );
  });

  testWidgets('neutral bubbles hug short text and distinguish own messages', (
    tester,
  ) async {
    for (final own in [false, true]) {
      await tester.pumpWidget(
        _testable(
          MessageContent(content: 'Hello', bubble: true, ownMessage: own),
        ),
      );
      await tester.pumpAndSettle();
      expect(bubbles, findsOneWidget);
      expect(tester.getSize(bubbles).width, lessThan(160));
      final decoration =
          tester.widget<Container>(bubbles).decoration! as ShapeDecoration;
      expect(decoration.color, isNotNull);
      expect(
        find.descendant(of: bubbles, matching: _findRich('Hello')),
        findsWidgets,
      );
    }
  });

  testWidgets('attachments stay outside bubbles and text order is preserved', (
    tester,
  ) async {
    await tester.pumpWidget(
      _testable(
        const MessageContent(
          content:
              'Before\n\n[file.txt](https://example.com/file.txt)\n\nAfter',
          tags: [
            ['imeta', 'url https://example.com/file.txt', 'm text/plain'],
          ],
          bubble: true,
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(bubbles, findsNWidgets(2));
    expect(
      find.descendant(of: bubbles, matching: _findRich('file.txt')),
      findsNothing,
    );
    expect(
      tester.getTopLeft(_findRich('Before')).dy,
      lessThan(tester.getTopLeft(_findRich('file.txt')).dy),
    );
    expect(
      tester.getTopLeft(_findRich('file.txt')).dy,
      lessThan(tester.getTopLeft(_findRich('After')).dy),
    );
  });

  testWidgets('fenced attachment examples remain in the text bubble', (
    tester,
  ) async {
    await tester.pumpWidget(
      _testable(
        const MessageContent(
          content: '```text\n![example](https://example.com/image.png)\n```',
          bubble: true,
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(bubbles, findsOneWidget);
    expect(
      find.descendant(
        of: bubbles,
        matching: _findRich('![example](https://example.com/image.png)'),
      ),
      findsWidgets,
    );
    expect(tester.takeException(), isNull);
  });

  testWidgets('bubble text reflows at narrow width and large text', (
    tester,
  ) async {
    _setSurfaceSize(tester, const Size(320, 800));
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await tester.pumpWidget(
      _testable(
        MediaQuery(
          data: const MediaQueryData(textScaler: TextScaler.linear(2)),
          child: const SizedBox(
            width: 230,
            child: MessageContent(
              content:
                  'A longer message that wraps across multiple lines without clipping.',
              bubble: true,
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(tester.getSize(bubbles).width, lessThanOrEqualTo(230));
    expect(tester.getSize(bubbles).height, greaterThan(80));
    expect(tester.takeException(), isNull);
  });
}

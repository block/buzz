import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/message_mention_pill.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('authored mention casing survives; bindings still resolve', (
    tester,
  ) async {
    String? tapped;
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: MessageContent(
          content: '@Helper and @ALICE',
          mentionNames: {
            'a' * 64: 'Helper',
            'b' * 64: 'Helper',
            'c' * 64: 'Alice',
          },
          onMentionTap: (key) => tapped = key,
        ),
      ),
    );
    await tester.pumpAndSettle();
    // Ambiguous plain namesake text keeps its authored casing.
    expect(find.textContaining('@Helper', findRichText: true), findsOneWidget);
    expect(find.textContaining('@helper', findRichText: true), findsNothing);
    // A bound mention matches case-insensitively and still resolves.
    await tester.tap(find.text('Alice'));
    expect(tapped, 'c' * 64);
  });

  testWidgets('signed names that fold alike leave a plain mention as text', (
    tester,
  ) async {
    final sam = 'd' * 64;
    final longS = 'e' * 64;
    for (final names in [
      {sam: 'Sam', longS: 'ſam'},
      {longS: 'ſam', sam: 'Sam'},
    ]) {
      String? tapped;
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: MessageContent(
            content: 'hi @Sam',
            mentionNames: names,
            tags: [
              for (final key in names.keys) ['p', key],
            ],
            onMentionTap: (key) => tapped = key,
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byType(MessageMentionPill), findsNothing, reason: '$names');
      await tester.tap(find.textContaining('@Sam', findRichText: true));
      expect(tapped, isNull, reason: '$names');
    }
  });

  testWidgets('a tagged mention keeps its pill before a suffix', (
    tester,
  ) async {
    final bob = 'd' * 64;
    // The composer keeps Bob's recipient when punctuation, an apostrophe, a
    // hyphen or a slash follows the chip; the bubble draws the same pill with
    // the suffix as plain text after it. A letter continues the name.
    for (final (content, suffix) in [
      ("@Bob's idea", "'s idea"),
      ('@Bob’s idea', '’s idea'),
      ('@Bob-bot', '-bot'),
      ('@Bob/Alice', '/Alice'),
      ('@Bob.', '.'),
    ]) {
      String? tapped;
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: MessageContent(
            content: content,
            mentionNames: {bob: 'Bob'},
            tags: [
              ['p', bob],
            ],
            onMentionTap: (key) => tapped = key,
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Bob'), findsOneWidget, reason: content);
      expect(
        find.textContaining(suffix, findRichText: true),
        findsOneWidget,
        reason: content,
      );
      await tester.tap(find.text('Bob'));
      expect(tapped, bob, reason: content);
    }
    String? tapped;
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: MessageContent(
          content: '@Bobby',
          mentionNames: {bob: 'Bob'},
          tags: [
            ['p', bob],
          ],
          onMentionTap: (key) => tapped = key,
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Bob'), findsNothing);
    await tester.tap(find.text('Bobby'));
    expect(tapped, isNull);
  });

  // Review of #9699: an italic or bold mention sent before underscores
  // counted as name characters draws its whole name as one pill, and a chip
  // the composer qualified because its suffix spelled another name opens
  // its own profile.
  testWidgets('emphasis and a qualified chip keep each pill on its owner', (
    tester,
  ) async {
    final kenny = 'd' * 64;
    final bob = 'b' * 64;
    final bobBot = 'e' * 64;
    for (final content in ['_@Kenny Lopez_', '__@Kenny Lopez__ hi']) {
      String? tapped;
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: MessageContent(
            content: content,
            mentionNames: {kenny: 'Kenny Lopez'},
            tags: [
              ['p', kenny],
            ],
            onMentionTap: (key) => tapped = key,
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Kenny Lopez'), findsOneWidget, reason: content);
      expect(
        find.textContaining('Lopez', findRichText: true),
        findsOneWidget,
        reason: content,
      );
      expect(find.textContaining('_', findRichText: true), findsNothing);
      await tester.tap(find.text('Kenny Lopez'));
      expect(tapped, kenny, reason: content);
    }
    final taps = <String>[];
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: MessageContent(
          content: '@Bob ($bob)-bot and @Bob-bot',
          mentionNames: {bob: 'Bob', bobBot: 'Bob-bot'},
          tags: [
            ['p', bob],
            ['p', bobBot],
          ],
          onMentionTap: taps.add,
        ),
      ),
    );
    await tester.pumpAndSettle();
    // A qualified pill reads the name with a short key, as namesakes do.
    await tester.tap(find.text('Bob (bbbbbbbb…bbbb)'));
    await tester.tap(find.text('Bob-bot'));
    expect(taps, [bob, bobBot]);
    expect(find.textContaining('-bot and', findRichText: true), findsOneWidget);
    expect(find.textContaining(bob, findRichText: true), findsNothing);
  });

  // Review of #9699: an edit saves a mention as an identity link, and its
  // pill is highlighted and tinted exactly as the tagged mention it replaced.
  testWidgets('an identity link pill is highlighted and tinted as a tagged '
      'mention is', (tester) async {
    final me = 'a' * 64;
    final bob = 'b' * 64;
    String link(String name, String key) =>
        '[@$name](nostr:'
        '${nostr.Nip19.encode(prefix: nostr.Nip19Prefix.npub, data: key)})';
    Future<(Color?, Color?)> pill(
      String content,
      String key, {
      required bool own,
      required bool tagged,
    }) async {
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [myPubkeyProvider.overrideWithValue(me)],
          child: MessageContent(
            key: UniqueKey(),
            content: content,
            bubble: true,
            ownMessage: own,
            mentionNames: {me: 'Me', bob: 'Bob'},
            tags: [
              if (tagged) ['p', key],
            ],
          ),
        ),
      );
      await tester.pumpAndSettle();
      final found = find.byType(MessageMentionPill);
      expect(found, findsOneWidget, reason: content);
      final fill = tester.widget<Container>(
        find.descendant(of: found, matching: find.byType(Container)),
      );
      final at = tester.widget<Text>(
        find.descendant(of: found, matching: find.text('@')),
      );
      return ((fill.decoration! as BoxDecoration).color, at.style?.color);
    }

    final looks = <String, (Color?, Color?)>{};
    for (final (name, key, own) in [
      // The viewer mentioned in someone else's bubble: the self highlight.
      ('Me', me, false),
      // A mention in the viewer's own bubble: the own-bubble tint.
      ('Bob', bob, true),
      ('Me', me, true),
      ('Bob', bob, false),
    ]) {
      final reason = '$name, own bubble: $own';
      final tagged = await pill('hi @$name', key, own: own, tagged: true);
      final linked = await pill(
        'hi ${link(name, key)}',
        key,
        own: own,
        tagged: false,
      );
      expect(linked, tagged, reason: reason);
      looks[reason] = linked;
    }
    // The cases differ, so the comparison above is not of one default look.
    expect(
      looks['Me, own bubble: false'],
      isNot(looks['Bob, own bubble: false']),
    );
    expect(
      looks['Bob, own bubble: true'],
      isNot(looks['Bob, own bubble: false']),
    );
  });

  for (final tagged in [false, true]) {
    testWidgets(
      'qualified chips preserve authority and narrow layout: $tagged',
      (tester) async {
        tester.view.physicalSize = const Size(320, 640);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        final semantics = tester.ensureSemantics();
        final first = 'a' * 64, second = 'b' * 64;
        String? tapped;
        await tester.pumpWidget(
          WidgetHelpers.testable(
            child: MediaQuery(
              data: const MediaQueryData(textScaler: TextScaler.linear(2)),
              child: MessageContent(
                content: '@Scout @Scout ($second)',
                mentionNames: {second: 'Scout', first: 'Scout'},
                tags: [
                  for (final key in [first, if (tagged) second]) ['p', key],
                ],
                onMentionTap: (key) => tapped = key,
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
        expect(find.text('Scout'), findsNothing);
        final qualified = find.text('Scout (bbbbbbbb…bbbb)');
        if (tagged) {
          await tester.tap(qualified);
          expect(tapped, second);
          expect(
            find.bySemanticsLabel(RegExp('Scout.*$second')),
            findsOneWidget,
          );
        } else {
          expect(qualified, findsNothing);
          expect(tapped, isNull);
        }
        expect(tester.takeException(), isNull);
        semantics.dispose();
      },
    );
  }
}

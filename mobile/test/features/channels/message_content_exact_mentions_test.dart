import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/message_mention_pubkeys.dart';
import 'package:buzz/shared/mentions/mention_tags.dart';
import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('DM recipients cannot supply a missing historical identity', (
    tester,
  ) async {
    final first = 'a' * 64, second = 'b' * 64, bob = 'c' * 64;
    final recipients = messageMentionPubkeys(
      channel: Channel(
        id: 'dm',
        name: 'DM',
        channelType: 'dm',
        visibility: 'private',
        description: '',
        createdBy: 'self',
        createdAt: DateTime(2026),
        memberCount: 4,
      ),
      senderPubkey: 'self',
      explicitMentions: [first, second],
      dmRecipientPubkeys: [first, second, bob],
    );
    final keys = mentionedPubkeysFromTags([
      for (final key in recipients) ['p', key],
    ]);
    // MessageBubble omits profiles with no displayName. Bob is only a DM
    // recipient, not the author-selected plain Scout.
    final profiles = {second: 'Scout', bob: 'Bob'};
    String? tapped;
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: MessageContent(
          content: '@Scout @Scout ($second)',
          mentionNames: {
            for (final key in keys)
              if (profiles[key] != null) key: profiles[key]!,
          },
          onMentionTap: (key) => tapped = key,
        ),
      ),
    );
    await tester.pumpAndSettle();
    if (find.text('Bob').evaluate().isNotEmpty) {
      await tester.tap(find.text('Bob'));
    }
    expect(tapped, isNot(bob));
    expect(find.text('Bob'), findsNothing);
    expect(find.text('Scout'), findsNothing);
    await tester.tap(find.text('Scout (bbbbbbbb…bbbb)'));
    expect(tapped, second);
  });

  testWidgets('untagged qualifiers cannot become shorter clickable aliases', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: MessageContent(
          content: '@Scout @Scout (${'b' * 64})',
          mentionNames: {'a' * 64: 'Scout'},
          onMentionTap: (_) => fail('unbound text is not a profile target'),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Scout'), findsNothing);
    expect(tester.takeException(), isNull);
  });

  testWidgets('qualified chips wrap in narrow layouts at large text sizes', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(320, 640);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: MediaQuery(
          data: const MediaQueryData(textScaler: TextScaler.linear(2)),
          child: MessageContent(
            content: '@Scout (${'b' * 64})',
            mentionNames: {'b' * 64: 'Scout'},
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
  });

  testWidgets('only qualified namesake chips have an exact historical target', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    final first = 'a' * 64;
    final second = 'b' * 64;
    String? tapped;
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: MessageContent(
          content: '@Scout @Scout ($second)',
          mentionNames: {second: 'Scout', first: 'Scout'},
          onMentionTap: (key) => tapped = key,
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Scout'), findsNothing);
    await tester.tap(find.text('Scout (bbbbbbbb…bbbb)'));
    expect(tapped, second);
    expect(find.bySemanticsLabel(RegExp('Scout.*$second')), findsOneWidget);
    expect(tester.takeException(), isNull);
    semantics.dispose();
  });
  testWidgets(
    'historical plain namesakes stay unbound with ordinary siblings',
    (tester) async {
      final first = 'a' * 64, second = 'b' * 64, sibling = 'c' * 64;
      for (final firstName in ['Scout', 'Renamed Scout', first]) {
        for (final secondName in ['Scout', 'Renamed Scout', second]) {
          for (final reverse in [false, true]) {
            for (final ambiguous in [false, true]) {
              final entries = {
                first: firstName,
                second: secondName,
                sibling: 'Alice',
                if (ambiguous) 'd' * 64: 'Unknown',
              }.entries.toList();
              String? tapped;
              await tester.pumpWidget(
                WidgetHelpers.testable(
                  child: MessageContent(
                    content: '@Scout @Scout ($second) @Alice',
                    mentionNames: Map.fromEntries(
                      reverse ? entries.reversed : entries,
                    ),
                    onMentionTap: (key) => tapped = key,
                  ),
                ),
              );
              await tester.pumpAndSettle();
              expect(find.text(firstName), findsNothing);
              await tester.tap(find.text('Scout (bbbbbbbb…bbbb)'));
              expect(tapped, second);
              await tester.tap(find.text('Alice'));
              expect(tapped, sibling);
            }
          }
        }
      }
    },
  );
}

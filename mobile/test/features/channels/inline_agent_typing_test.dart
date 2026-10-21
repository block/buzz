import 'package:buzz/features/channels/channel_identity_names_provider.dart';
import 'package:buzz/features/channels/agent_typing_bubble.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/features/channels/channel_typing_provider.dart';
import 'package:buzz/features/channels/inline_agent_typing.dart';
import 'package:buzz/shared/identity_names/identity_names.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const agent =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const profile = UserProfile(pubkey: agent, displayName: 'Test agent');

class Profiles extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => {agent: profile};
}

Widget surface({
  bool reduced = false,
  bool typing = true,
  bool bot = true,
  List<TypingEntry>? entries,
  double textScale = 1,
}) => ProviderScope(
  overrides: [
    agentMentionPubkeysProvider(
      'chat',
    ).overrideWithValue(bot ? {agent} : <String>{}),
    userCacheProvider.overrideWith(Profiles.new),
    channelIdentityNamesProvider('chat').overrideWithValue(
      const IdentityNameSources(profiles: {agent: profile}).scope([agent]),
    ),
  ],
  child: MaterialApp(
    theme: AppTheme.light(),
    home: MediaQuery(
      data: MediaQueryData(
        disableAnimations: reduced,
        textScaler: TextScaler.linear(textScale),
      ),
      child: Scaffold(
        body: InlineAgentTyping(
          channelId: 'chat',
          entries:
              entries ??
              [if (typing) const TypingEntry(pubkey: agent, expiresAtMs: 8000)],
        ),
      ),
    ),
  ),
);
void main() {
  testWidgets(
    'message rail, author and dots replace status text without a timestamp',
    (tester) async {
      await tester.pumpWidget(surface());
      expect(find.text('Test agent'), findsOneWidget);
      expect(find.byType(AvatarImage), findsOneWidget);
      expect(
        tester.widget<AvatarImage>(find.byType(AvatarImage)).isAgent,
        isTrue,
      );
      expect(find.textContaining('typing'), findsNothing);
      expect(find.byType(Text), findsNWidgets(2)); // Name and avatar initial.
      expect(find.byKey(const ValueKey('agent-typing-dot-2')), findsOneWidget);
      expect(find.byKey(const ValueKey('agent-typing-dot-0')), findsOneWidget);
      final dot = find.byKey(const ValueKey('agent-typing-dot-0'));
      final initial = tester
          .widget<Transform>(dot)
          .transform
          .getTranslation()
          .y;
      await tester.pump(const Duration(milliseconds: 240));
      expect(
        tester.widget<Transform>(dot).transform.getTranslation().y,
        lessThan(initial),
      );
      expect(
        tester.getTopLeft(find.text('Test agent')).dx,
        messageAvatarSize + messageAvatarContentGap + Grid.xs,
      );
      await tester.pumpWidget(surface(typing: false));
      expect(find.byType(AvatarImage), findsNothing);
      expect(find.text('Working…'), findsNothing);
    },
  );
  testWidgets('human typing keeps a round avatar', (tester) async {
    final semantics = tester.ensureSemantics();
    await tester.pumpWidget(surface(bot: false));
    expect(find.bySemanticsLabel('Test agent is typing'), findsOneWidget);
    expect(
      tester.widget<AvatarImage>(find.byType(AvatarImage)).isAgent,
      isFalse,
    );
    expect(find.byKey(const ValueKey('agent-typing-dot-2')), findsOneWidget);
    semantics.dispose();
  });
  testWidgets('many typers keep rows and active animation owners bounded', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    final entries = List.generate(
      1024,
      (index) => TypingEntry(
        pubkey: index.toRadixString(16).padLeft(64, '0'),
        expiresAtMs: 8000,
      ),
    );
    // The first three stay visible while the aggregate grows. Duplicate scopes
    // for the same identity must not inflate the remainder count.
    for (final count in [3, 4, 1024]) {
      final visibleEntries = entries.take(count).toList();
      await tester.pumpWidget(
        surface(
          entries: [
            ...visibleEntries,
            TypingEntry(
              pubkey: visibleEntries.last.pubkey.toUpperCase(),
              threadHeadId: 'thread',
              expiresAtMs: 8000,
            ),
          ],
          textScale: 2,
        ),
      );
      await tester.pump(const Duration(milliseconds: 240));
      expect(find.byType(AvatarImage), findsNWidgets(3));
      expect(find.byType(AgentTypingBubble), findsNWidgets(3));
      // Each repeating ticker schedules one transient callback for the next
      // frame: this measures active work, not just visible dot widgets.
      expect(tester.binding.transientCallbackCount, 3);
      for (final entry in entries.take(3)) {
        expect(
          find.byKey(ValueKey('inline-agent-typing-${entry.pubkey}')),
          findsOneWidget,
        );
      }
      if (count > 3) {
        final remainder = count - 3;
        expect(
          find.text('and $remainder ${remainder == 1 ? 'other' : 'others'}'),
          findsOneWidget,
        );
        expect(
          find.bySemanticsLabel(
            '$remainder ${remainder == 1 ? 'other person is' : 'other people are'} typing',
          ),
          findsOneWidget,
        );
      } else {
        expect(find.textContaining('and '), findsNothing);
      }
      expect(tester.takeException(), isNull);
    }
    await tester.pumpWidget(surface(entries: entries.take(1).toList()));
    await tester.pump(const Duration(milliseconds: 240));
    expect(find.byType(AvatarImage), findsOneWidget);
    expect(find.textContaining('and '), findsNothing);
    expect(tester.binding.transientCallbackCount, 1);
    await tester.pumpWidget(surface(entries: entries, reduced: true));
    await tester.pumpAndSettle();
    expect(find.byType(AgentTypingBubble), findsNWidgets(3));
    expect(tester.binding.transientCallbackCount, 0);
    await tester.pumpWidget(surface(typing: false, reduced: true));
    await tester.pumpAndSettle();
    expect(find.byType(AvatarImage), findsNothing);
    expect(tester.binding.transientCallbackCount, 0);
    semantics.dispose();
  });
  testWidgets('Reduce Motion leaves static dots and no active animation', (
    tester,
  ) async {
    await tester.pumpWidget(surface(reduced: true));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('agent-typing-dot-2')), findsOneWidget);
    expect(find.byType(ShaderMask), findsNothing);
    expect(tester.binding.hasScheduledFrame, isFalse);
  });
}

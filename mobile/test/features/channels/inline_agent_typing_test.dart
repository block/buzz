import 'package:buzz/features/channels/channel_identity_names_provider.dart';
import 'package:buzz/features/channels/agent_typing_bubble.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/features/channels/channel_typing_provider.dart';
import 'package:buzz/features/channels/inline_agent_typing.dart';
import 'package:buzz/features/channels/typing_motion_preview.dart';
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

class DemoProfiles extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => {
    for (var i = 0; i < 8; i++)
      'person-$i': UserProfile(pubkey: 'person-$i', displayName: 'Person $i'),
    agent: profile,
  };
}

Widget surface({
  bool preview = false,
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
    userCacheProvider.overrideWith(preview ? DemoProfiles.new : Profiles.new),
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
        body: preview
            ? const TypingMotionPreview(channelId: 'chat')
            : InlineAgentTyping(
                channelId: 'chat',
                entries:
                    entries ??
                    [
                      if (typing)
                        const TypingEntry(pubkey: agent, expiresAtMs: 8000),
                    ],
              ),
      ),
    ),
  ),
);
void main() {
  testWidgets('one to two keeps the gentle straight repositioning', (
    tester,
  ) async {
    final entries = [
      for (final key in [agent, 'second'])
        TypingEntry(pubkey: key, expiresAtMs: 8000),
    ];
    final first = find.byKey(const ValueKey('inline-agent-typing-$agent'));
    await tester.pumpWidget(surface(entries: entries.take(1).toList()));
    final start = tester.getCenter(first);
    await tester.pumpWidget(surface(entries: entries));
    await tester.pump(const Duration(milliseconds: 100));
    final during = tester.getCenter(first);
    await tester.pump(const Duration(milliseconds: 600));
    final end = tester.getCenter(first);
    final travel = end - start;
    final offset = during - start;
    expect(
      (travel.dx * offset.dy - travel.dy * offset.dx).abs() / travel.distance,
      lessThan(0.01),
    );
  });
  testWidgets('demo still shows a squircle when agent discovery is empty', (
    tester,
  ) async {
    await tester.pumpWidget(surface(preview: true, bot: false));
    await tester.tap(find.byTooltip('More people'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 700));
    final avatars = tester
        .widgetList<AvatarImage>(find.byType(AvatarImage))
        .toList();
    expect(avatars, hasLength(2));
    expect(avatars[0].isAgent, isFalse);
    expect(avatars[1].isAgent, isTrue);
    expect(find.textContaining('demo agents'), findsOneWidget);
    await tester.pumpWidget(const SizedBox.shrink());
  });
  testWidgets('first avatar is immediately still and visible', (tester) async {
    await tester.pumpWidget(surface());
    final avatar = find.byType(AvatarImage);
    final start = tester.getRect(avatar);
    await tester.pump(const Duration(milliseconds: 100));
    expect(tester.getRect(avatar), start);
    final opacity = tester.widget<Opacity>(
      find.ancestor(of: avatar, matching: find.byType(Opacity)).first,
    );
    expect(opacity.opacity, 1);
  });
  testWidgets(
    'third arrival moves existing avatars directly without overshoot',
    (tester) async {
      final entries = [
        for (final key in [agent, 'second', 'third'])
          TypingEntry(pubkey: key, expiresAtMs: 8000),
      ];
      final first = find.byKey(const ValueKey('inline-agent-typing-$agent'));
      await tester.pumpWidget(surface(entries: entries.take(2).toList()));
      await tester.pump(const Duration(milliseconds: 700));
      final start = tester.getCenter(first);
      await tester.pumpWidget(surface(entries: entries));
      await tester.pump(const Duration(milliseconds: 100));
      final during = tester.getCenter(first);
      final samples = <Offset>[during];
      for (var frame = 0; frame < 8; frame++) {
        await tester.pump(const Duration(milliseconds: 40));
        samples.add(tester.getCenter(first));
      }
      await tester.pump(const Duration(milliseconds: 280));
      final end = tester.getCenter(first);
      final direction = end - start;
      var previousProgress = 0.0;
      for (final sample in samples) {
        final delta = sample - start;
        final progress =
            (delta.dx * direction.dx + delta.dy * direction.dy) /
            direction.distanceSquared;
        expect(progress, inInclusiveRange(previousProgress - 0.001, 1.001));
        previousProgress = progress;
      }
      final travel = end - start;
      final offset = during - start;
      final perpendicularDistance =
          (travel.dx * offset.dy - travel.dy * offset.dx).abs() /
          travel.distance;
      expect(perpendicularDistance, lessThan(0.01));
      expect(find.byType(AgentTypingBubble), findsOneWidget);
    },
  );
  testWidgets(
    'demo includes an agent alongside a person even after eight cached people',
    (tester) async {
      await tester.pumpWidget(surface(preview: true));
      await tester.tap(find.byTooltip('More people'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 700));
      final avatars = tester
          .widgetList<AvatarImage>(find.byType(AvatarImage))
          .toList();
      expect(avatars, hasLength(2));
      expect(avatars[0].isAgent, isFalse);
      expect(avatars[1].isAgent, isTrue);
      expect(find.byIcon(Icons.smart_toy_rounded), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );
  testWidgets('overflow stays circular even when replacing an agent', (
    tester,
  ) async {
    final entries = [
      for (final key in ['first', 'second', 'third', agent, 'fifth'])
        TypingEntry(pubkey: key, expiresAtMs: 8000),
    ];
    await tester.pumpWidget(surface(entries: entries.take(4).toList()));
    await tester.pump(const Duration(milliseconds: 700));
    final avatarRect = tester.getRect(
      find.byKey(const ValueKey('inline-agent-typing-$agent')),
    );
    await tester.pumpWidget(surface(entries: entries));
    await tester.pump(const Duration(milliseconds: 700));
    final badge = find.byKey(const ValueKey('typing-avatar-overflow'));
    final badgeRect = tester.getRect(badge);
    expect(badgeRect.width, greaterThan(avatarRect.width));
    expect(badgeRect.size, const Size(26, 26));
    expect(badgeRect.bottomRight, avatarRect.bottomRight);
    final decoration =
        tester.widget<Container>(badge).decoration! as BoxDecoration;
    expect(decoration.shape, BoxShape.circle);
    expect(decoration.borderRadius, isNull);
    expect(decoration.border, isNull);
    expect(find.text('+2'), findsOneWidget);
  });
  testWidgets('avatar layouts stay in one footprint through joins and leaves', (
    tester,
  ) async {
    final entries = List.generate(
      5,
      (i) => TypingEntry(
        pubkey: i.toRadixString(16).padLeft(64, '0'),
        expiresAtMs: 8000,
      ),
    );
    for (final count in [1, 2, 3, 4, 5, 4, 3, 2, 1]) {
      await tester.pumpWidget(surface(entries: entries.take(count).toList()));
      await tester.pump(const Duration(milliseconds: 700));
      final cluster = tester.getRect(
        find.byKey(const ValueKey('typing-avatar-cluster')),
      );
      expect(cluster.size, const Size(messageAvatarSize, messageAvatarSize));
      final avatars = find.byType(AvatarImage);
      expect(avatars, findsNWidgets(count > 4 ? 3 : count));
      expect(find.byType(AgentTypingBubble), findsOneWidget);
      final rects = [
        for (var i = 0; i < (count > 4 ? 3 : count); i++)
          tester.getRect(avatars.at(i)),
      ];
      for (final rect in rects) {
        expect(cluster.contains(rect.center), isTrue);
      }
      if (count == 2) {
        expect(rects[0].left, lessThan(rects[1].left));
        expect(rects[0].top, lessThan(rects[1].top));
      }
      if (count == 3) {
        expect(rects[0].center.dx, cluster.center.dx);
        expect(rects[1].top, rects[2].top);
        expect(rects[0].top, lessThan(rects[1].top));
      }
      if (count == 4) {
        expect(rects[0].top, rects[1].top);
        expect(rects[2].top, rects[3].top);
        expect(rects[0].left, rects[2].left);
        expect(rects[1].left, rects[3].left);
      }
      if (count == 5) expect(find.text('+2'), findsOneWidget);
      expect(tester.takeException(), isNull);
    }
  });
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
      await tester.pump(const Duration(milliseconds: 700));
      expect(find.byType(AvatarImage), findsNWidgets(count == 4 ? 4 : 3));
      expect(find.byType(AgentTypingBubble), findsOneWidget);
      // Each repeating ticker schedules one transient callback for the next
      // frame: this measures active work, not just visible dot widgets.
      expect(tester.binding.transientCallbackCount, 1);
      for (final entry in entries.take(3)) {
        expect(
          find.byKey(ValueKey('inline-agent-typing-${entry.pubkey}')),
          findsOneWidget,
        );
      }
      if (count > 4) {
        final remainder = count - 3;
        expect(find.text('+$remainder'), findsOneWidget);
        expect(
          find.bySemanticsLabel(RegExp('.*and ${count - 1} others are typing')),
          findsOneWidget,
        );
      }
      expect(tester.takeException(), isNull);
    }
    await tester.pumpWidget(surface(entries: entries.take(1).toList()));
    await tester.pump(const Duration(milliseconds: 700));
    expect(find.byType(AvatarImage), findsOneWidget);
    expect(find.textContaining('and '), findsNothing);
    expect(tester.binding.transientCallbackCount, 1);
    await tester.pumpWidget(surface(entries: entries, reduced: true));
    await tester.pumpAndSettle();
    expect(find.byType(AgentTypingBubble), findsOneWidget);
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

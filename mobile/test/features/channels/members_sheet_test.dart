import 'package:buzz/features/channels/agent_activity/working_bots_provider.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_identity_names_provider.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/members_sheet.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/features/profile/user_status_cache_provider.dart';
import 'package:buzz/shared/identity_names/identity_names.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _Cache extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => {};
  @override
  Future<bool> preload(List<String> pubkeys) async => true;
}

class _Statuses extends UserStatusCacheNotifier {
  @override
  Map<String, UserStatus?> build() => {};
  @override
  void track(List<String> pubkeys) {}
}

String _key(int index) => index.toRadixString(16).padLeft(64, '0');

Future<void> _pump(
  WidgetTester tester,
  List<ChannelMember> members, {
  ValueChanged<String>? onTap,
}) async {
  final names = IdentityNameSources.empty.scope(
    members.map((m) => m.pubkey),
    fallbackNames: {for (final m in members) m.pubkey: m.displayName!},
  );
  await tester.pumpWidget(
    ProviderScope(
      overrides: [
        channelMembersProvider('channel').overrideWith((ref) async => members),
        channelIdentityNamesProvider('channel').overrideWith((ref) => names),
        userCacheProvider.overrideWith(_Cache.new),
        userStatusCacheProvider.overrideWith(_Statuses.new),
        workingBotPubkeysProvider('channel').overrideWith((ref) => {}),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: Align(
            alignment: Alignment.bottomCenter,
            child: MembersSheet(
              channel: Channel(
                id: 'channel',
                name: 'Test',
                channelType: 'stream',
                visibility: 'open',
                description: '',
                createdBy: _key(0),
                createdAt: DateTime(2026),
                memberCount: members.length,
              ),
              currentPubkey: _key(0),
              onMemberTap: (_, key) => onTap?.call(key),
            ),
          ),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
}

List<ChannelMember> _roster(int people, int bots) => [
  for (var i = 0; i < people + bots; i++)
    ChannelMember(
      pubkey: _key(i),
      role: i == 0
          ? 'owner'
          : i < people
          ? 'member'
          : 'bot',
      joinedAt: DateTime(2026),
      displayName: i < people ? 'Person $i' : 'Agent $i',
    ),
];

void main() {
  testWidgets('large groups mount nearby rows and can reach the final agent', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await _pump(tester, _roster(100, 100));
    expect(find.text('People · 100'), findsOneWidget);
    expect(find.text('You'), findsOneWidget);
    expect(find.byType(ListTile).evaluate().length, lessThan(20));
    expect(find.text('Person 99'), findsNothing);
    final scrollable = find.descendant(
      of: find.byKey(const ValueKey('members-sheet-list')),
      matching: find.byType(Scrollable),
    );
    await tester.scrollUntilVisible(
      find.text('Person 99'),
      300,
      scrollable: scrollable,
      maxScrolls: 100,
    );
    expect(find.text('Person 99'), findsOneWidget);
    await tester.scrollUntilVisible(
      find.text('Agent 199'),
      300,
      scrollable: scrollable,
      maxScrolls: 100,
    );
    expect(find.text('Agent 199'), findsOneWidget);
    expect(find.byType(ListTile).evaluate().length, lessThan(20));
    expect(tester.takeException(), isNull);
  });

  testWidgets('short list keeps grouping, self label and person tap identity', (
    tester,
  ) async {
    String? selected;
    final semantics = tester.ensureSemantics();

    await _pump(tester, _roster(2, 1), onTap: (key) => selected = key);
    expect(find.text('People · 2'), findsOneWidget);
    expect(find.text('Agents · 1'), findsOneWidget);
    final agentIndex = tester.widget<IndexedSemantics>(
      find.ancestor(
        of: find.text('Agent 2'),
        matching: find.byType(IndexedSemantics),
      ),
    );
    expect(agentIndex.index, 2);
    expect(
      tester
          .widget<CustomScrollView>(
            find.byKey(const ValueKey('members-sheet-list')),
          )
          .semanticChildCount,
      3,
    );
    expect(find.text('You'), findsOneWidget);
    expect(find.bySemanticsLabel(RegExp('Person 1')), findsWidgets);
    await tester.tap(find.text('Person 1'));
    expect(selected, _key(1));
    semantics.dispose();
    expect(
      tester.getSize(find.byKey(const ValueKey('members-sheet-list'))).height,
      lessThan(400),
    );
    expect(tester.takeException(), isNull);
  });

  testWidgets('owner actions stay available after lazy scrolling', (
    tester,
  ) async {
    await _pump(tester, _roster(40, 1));
    final scrollable = find.descendant(
      of: find.byKey(const ValueKey('members-sheet-list')),
      matching: find.byType(Scrollable),
    );
    await tester.scrollUntilVisible(
      find.text('Person 39'),
      300,
      scrollable: scrollable,
      maxScrolls: 50,
    );
    final row = find.ancestor(
      of: find.text('Person 39'),
      matching: find.byType(ListTile),
    );
    await tester.tap(
      find.descendant(of: row, matching: find.byIcon(BuzzIcons.ellipsis)),
    );
    await tester.pumpAndSettle();
    expect(find.text('Role'), findsOneWidget);
    expect(find.text('Remove from channel'), findsOneWidget);
    // Inspect only: do not invoke a membership mutation.
    Navigator.of(tester.element(find.text('Role'))).pop();
    await tester.pumpAndSettle();
    await tester.scrollUntilVisible(
      find.text('Agent 40'),
      100,
      scrollable: scrollable,
    );
    final agent = find.ancestor(
      of: find.text('Agent 40'),
      matching: find.byType(ListTile),
    );
    await tester.tap(
      find.descendant(of: agent, matching: find.byIcon(BuzzIcons.ellipsis)),
    );
    await tester.pumpAndSettle();
    expect(find.text('View activity'), findsOneWidget);
    expect(find.text('Role'), findsNothing);
    expect(tester.takeException(), isNull);
  });

  testWidgets('empty roster retains empty message', (tester) async {
    await _pump(tester, []);
    expect(find.text('No members found.'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}

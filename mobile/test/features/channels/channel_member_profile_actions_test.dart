import 'dart:async';
import 'dart:ui' show SemanticsAction;

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channel_member_profile_actions.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _Channels extends ChannelsNotifier {
  _Channels(this.channel);
  final Channel channel;
  @override
  Future<List<Channel>> build() async => [channel];
}

class _Actions extends Fake implements ChannelActions {
  final calls = <String>[];
  Completer<void>? pending;
  bool fail = false;

  @override
  Future<void> changeMemberRole({
    required String channelId,
    required String pubkey,
    required String role,
  }) async {
    calls.add('$channelId:$pubkey:$role');
    if (pending != null) await pending!.future;
    if (fail) throw Exception('rejected');
  }

  @override
  Future<void> removeMember({
    required String channelId,
    required String pubkey,
  }) async {
    calls.add('$channelId:$pubkey:remove');
    if (fail) throw Exception('rejected');
  }
}

Channel _channel({bool dm = false, bool archived = false}) => Channel(
  id: 'test',
  name: 'General',
  channelType: dm ? 'dm' : 'stream',
  visibility: 'private',
  description: '',
  createdBy: 'self',
  createdAt: DateTime(2025),
  memberCount: 3,
  archivedAt: archived ? DateTime(2025) : null,
);
ChannelMember _member(String pubkey, String role) =>
    ChannelMember(pubkey: pubkey, role: role, joinedAt: DateTime(2025));

Future<ProviderContainer> _pump(
  WidgetTester tester, {
  String actorRole = 'owner',
  String targetRole = 'member',
  String target = 'alice',
  bool dm = false,
  bool archived = false,
  bool inSheet = false,
  _Actions? actions,
  Future<List<ChannelMember>> Function()? load,
}) async {
  final channel = _channel(dm: dm, archived: archived);
  await tester.pumpWidget(
    ProviderScope(
      overrides: [
        currentPubkeyProvider.overrideWithValue('self'),
        channelsProvider.overrideWith(() => _Channels(channel)),
        channelMembersProvider('test').overrideWith(
          (ref) async => load == null
              ? [_member('self', actorRole), _member('alice', targetRole)]
              : await load(),
        ),
        channelActionsProvider.overrideWithValue(actions ?? _Actions()),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: inSheet
              ? Builder(
                  builder: (context) => TextButton(
                    onPressed: () => showModalBottomSheet<void>(
                      context: context,
                      builder: (_) => ChannelMemberProfileActions(
                        channel: channel,
                        pubkey: target,
                      ),
                    ),
                    child: const Text('Open profile'),
                  ),
                )
              : ChannelMemberProfileActions(channel: channel, pubkey: target),
        ),
      ),
    ),
  );
  await tester.pump();
  if (inSheet) {
    await tester.tap(find.text('Open profile'));
    await tester.pumpAndSettle();
  }
  return ProviderScope.containerOf(
    tester.element(find.byType(ChannelMemberProfileActions)),
  );
}

void main() {
  testWidgets('successful removal closes the member sheet', (tester) async {
    final actions = _Actions();
    await _pump(tester, actions: actions, inSheet: true);
    await tester.tap(find.text('Remove from channel'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Remove'));
    await tester.pumpAndSettle();
    expect(actions.calls, ['test:alice:remove']);
    expect(find.byType(ChannelMemberProfileActions), findsNothing);
    expect(find.text('Open profile'), findsOneWidget);
  });
  testWidgets('role action follows the refreshed roster', (tester) async {
    var role = 'member';
    final actions = _Actions();
    final container = await _pump(
      tester,
      actions: actions,
      load: () async => [_member('self', 'owner'), _member('alice', role)],
    );
    await tester.tap(find.text('Make channel admin'));
    await tester.pumpAndSettle();
    role = 'admin';
    container.invalidate(channelMembersProvider('test'));
    await tester.pumpAndSettle();
    expect(find.text('Make channel admin'), findsNothing);
    await tester.tap(find.text('Change to member'));
    await tester.pumpAndSettle();
    expect(actions.calls, ['test:alice:admin', 'test:alice:member']);
  });
  for (final role in ['member', 'admin', 'guest', 'bot']) {
    testWidgets('management rows for $role', (tester) async {
      await _pump(tester, targetRole: role);
      expect(
        find.text('Make channel admin'),
        role == 'member' || role == 'guest' ? findsOneWidget : findsNothing,
      );
      expect(
        find.text('Change to member'),
        role == 'admin' ? findsOneWidget : findsNothing,
      );
      expect(
        find.text('Make member'),
        role == 'guest' ? findsOneWidget : findsNothing,
      );
      expect(find.text('Remove from channel'), findsOneWidget);
    });
  }
  for (final scenario in [
    'member viewer',
    'self',
    'owner',
    'dm',
    'archived',
    'missing',
    'loading',
    'error',
  ]) {
    testWidgets('hides management for $scenario', (tester) async {
      await _pump(
        tester,
        actorRole: scenario == 'member viewer' ? 'member' : 'admin',
        targetRole: scenario == 'owner' ? 'owner' : 'member',
        target: scenario == 'self'
            ? 'self'
            : scenario == 'missing'
            ? 'absent'
            : 'alice',
        dm: scenario == 'dm',
        archived: scenario == 'archived',
        load: scenario == 'loading'
            ? () => Completer<List<ChannelMember>>().future
            : scenario == 'error'
            ? () => Future.error(Exception('offline'))
            : null,
      );
      expect(find.text('Make channel admin'), findsNothing);
      expect(find.text('Remove from channel'), findsNothing);
    });
  }
  testWidgets('role updates block duplicates and show rejection', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    try {
      final actions = _Actions()
        ..pending = Completer<void>()
        ..fail = true;
      await _pump(tester, actions: actions);
      await tester.tap(find.text('Make channel admin'));
      await tester.pump();
      final pending = find.bySemanticsLabel('Updating channel member');
      expect(pending, findsOneWidget);
      expect(
        tester
            .getSemantics(pending)
            .getSemanticsData()
            .flagsCollection
            .isLiveRegion,
        isTrue,
      );
      expect(find.bySemanticsLabel('Updating…'), findsNothing);
      for (final label in ['Make channel admin', 'Remove from channel']) {
        expect(
          tester
              .getSemantics(find.text(label))
              .getSemanticsData()
              .hasAction(SemanticsAction.tap),
          isFalse,
        );
      }
      await tester.tap(find.text('Make channel admin'));
      expect(actions.calls, ['test:alice:admin']);
      actions.pending!.complete();
      await tester.pumpAndSettle();
      expect(
        find.text('Could not change this role. Please try again.'),
        findsOneWidget,
      );
      expect(pending, findsNothing);
      final error = find.bySemanticsLabel(
        'Could not change this role. Please try again.',
      );
      expect(error, findsOneWidget);
      expect(
        tester
            .getSemantics(error)
            .getSemanticsData()
            .flagsCollection
            .isLiveRegion,
        isTrue,
      );
      expect(
        tester
            .getSemantics(find.text('Make channel admin'))
            .getSemanticsData()
            .hasAction(SemanticsAction.tap),
        isTrue,
      );
    } finally {
      semantics.dispose();
    }
  });
  testWidgets('removal requires confirmation and reports rejection', (
    tester,
  ) async {
    final actions = _Actions()..fail = true;
    await _pump(tester, actions: actions);
    await tester.tap(find.text('Remove from channel'));
    await tester.pumpAndSettle();
    expect(actions.calls, isEmpty);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(actions.calls, isEmpty);
    await tester.tap(find.text('Remove from channel'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Remove'));
    await tester.pumpAndSettle();
    expect(actions.calls, ['test:alice:remove']);
    expect(
      find.text('Could not remove this person. Please try again.'),
      findsOneWidget,
    );
  });
  testWidgets('revoked authority while confirming never submits removal', (
    tester,
  ) async {
    var role = 'admin';
    final actions = _Actions();
    final container = await _pump(
      tester,
      actions: actions,
      load: () async => [_member('self', role), _member('alice', 'member')],
    );
    await tester.tap(find.text('Remove from channel'));
    await tester.pumpAndSettle();
    role = 'member';
    container.invalidate(channelMembersProvider('test'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Remove'));
    await tester.pumpAndSettle();
    expect(actions.calls, isEmpty);
  });
}

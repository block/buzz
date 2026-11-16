import 'package:buzz/features/channels/add_members_sheet.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/recipient_directory_session.dart';

const _dmId = 'dm-1';
final _agent = 'f' * 64;
final _person = 'e' * 64;

/// Listed in the agent directory, but with no verified owner.
final _unowned = 'd' * 64;

/// The viewer and one other person.
const _twoPeople = {'self', 'other'};

Channel _dm(String id) => Channel(
  id: id,
  name: id,
  channelType: 'dm',
  visibility: 'private',
  description: '',
  createdBy: 'self',
  createdAt: DateTime(2026),
  memberCount: 3,
);

class _DmAdds extends Fake implements ChannelActions {
  _DmAdds(this.landsIn, {this.unconfirmed = 0});

  final String landsIn;

  /// How many times the DM the add lands in fails to confirm.
  int unconfirmed;
  final calls = <({List<String> pubkeys, bool includeHistory})>[];
  final opens = <String>[];

  void _confirm() {
    if (unconfirmed > 0) {
      unconfirmed--;
      throw DmAddUnconfirmed(landsIn);
    }
  }

  @override
  Future<Channel> openAddedDm(String channelId) async {
    opens.add(channelId);
    _confirm();
    return _dm(channelId);
  }

  @override
  Future<Channel> addToDm({
    required String channelId,
    required List<String> pubkeys,
    required bool includeHistory,
  }) async {
    expect(channelId, _dmId);
    calls.add((pubkeys: pubkeys, includeHistory: includeHistory));
    _confirm();
    return _dm(landsIn);
  }
}

/// Opens the DM add sheet, picks [name], and taps Add.
Future<List<Channel>> _add(
  WidgetTester tester,
  _DmAdds actions,
  String name, {
  Set<String> existing = _twoPeople,
}) async {
  final movedTo = <Channel>[];
  await tester.pumpWidget(
    ProviderScope(
      overrides: [
        channelActionsProvider.overrideWithValue(actions),
        channelMembersProvider(_dmId).overrideWith((ref) async => const []),
        agentOwnersProvider.overrideWith((ref) async => {_agent: _person}),
        relaySessionProvider.overrideWith(
          () => RecipientDirectorySession(
            {_agent: 'Scout', _person: 'Robin', _unowned: 'Wren'},
            agents: {_agent, _unowned},
          ),
        ),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: AddChannelMembersSheet(
            channelId: _dmId,
            existingPubkeys: existing,
            isDm: true,
            onAddedToDm: movedTo.add,
          ),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
  await tester.enterText(
    find.byKey(const ValueKey('add-channel-members-search')),
    name,
  );
  await tester.pump(const Duration(milliseconds: 300));
  await tester.pumpAndSettle();
  await tester.tap(find.textContaining(name).last);
  await tester.pumpAndSettle();
  await tester.tap(find.byKey(const ValueKey('add-channel-members-submit')));
  await tester.pumpAndSettle();
  return movedTo;
}

void main() {
  testWidgets('an agent joins the DM with its history without asking', (
    tester,
  ) async {
    final actions = _DmAdds(_dmId);
    final movedTo = await _add(tester, actions, 'Scout');

    expect(find.text('Include history'), findsNothing);
    expect(actions.calls.single.pubkeys, [_agent]);
    expect(actions.calls.single.includeHistory, isTrue);
    expect(movedTo.single.id, _dmId);
  });

  testWidgets('a directory listing without a verified owner is asked about', (
    tester,
  ) async {
    final actions = _DmAdds(_dmId);
    await _add(tester, actions, 'Wren');

    expect(find.text('Add Wren?'), findsOneWidget);
    expect(actions.calls, isEmpty);
  });

  testWidgets('adding a person asks, and Include history keeps this DM', (
    tester,
  ) async {
    final actions = _DmAdds(_dmId);
    final movedTo = await _add(tester, actions, 'Robin');

    expect(find.text('Add Robin?'), findsOneWidget);
    expect(find.text('Adding…'), findsNothing);
    expect(actions.calls, isEmpty);
    await tester.tap(find.text('Include history'));
    await tester.pumpAndSettle();

    expect(actions.calls.single.pubkeys, [_person]);
    expect(actions.calls.single.includeHistory, isTrue);
    // The grown DM is handed back, so its details show the new members.
    expect(movedTo.single.id, _dmId);
  });

  testWidgets('Start a new DM adds without history and opens the new DM', (
    tester,
  ) async {
    final actions = _DmAdds('dm-2');
    final movedTo = await _add(tester, actions, 'Robin');

    await tester.tap(find.text('Start a new DM'));
    await tester.pumpAndSettle();

    expect(actions.calls.single.includeHistory, isFalse);
    expect(movedTo.single.id, 'dm-2');
  });

  testWidgets('a person added to a DM of one person starts a new DM', (
    tester,
  ) async {
    final actions = _DmAdds('dm-2');
    final movedTo = await _add(tester, actions, 'Robin', existing: {'self'});

    expect(find.text('Include history'), findsNothing);
    expect(actions.calls.single.pubkeys, [_person]);
    expect(actions.calls.single.includeHistory, isFalse);
    expect(movedTo.single.id, 'dm-2');
  });

  testWidgets('cancelling the choice adds no one', (tester) async {
    final actions = _DmAdds(_dmId);
    await _add(tester, actions, 'Robin');

    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    expect(actions.calls, isEmpty);
    expect(
      tester
          .widget<FilledButton>(
            find.byKey(const ValueKey('add-channel-members-submit')),
          )
          .onPressed,
      isNotNull,
    );
  });

  testWidgets(
    'an accepted add that is not confirmed yet is opened, not resent',
    (tester) async {
      final actions = _DmAdds('dm-2', unconfirmed: 2);
      final movedTo = await _add(tester, actions, 'Robin');
      await tester.tap(find.text('Start a new DM'));
      await tester.pumpAndSettle();

      expect(find.text('Added. Tap Add again to open the DM.'), findsOneWidget);
      expect(movedTo, isEmpty);

      // Add only looks for the DM again, until it shows.
      for (var attempt = 0; attempt < 2; attempt++) {
        await tester.tap(
          find.byKey(const ValueKey('add-channel-members-submit')),
        );
        await tester.pumpAndSettle();
      }

      expect(actions.calls, hasLength(1));
      expect(actions.opens, ['dm-2', 'dm-2']);
      expect(movedTo.single.id, 'dm-2');
    },
  );
}

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

/// Claims to be an agent, with an owner, but the relay counts them as a
/// person.
final _claimsAgent = 'c' * 64;

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

/// Who the relay counts as an agent. Only it knows, so this app never does.
final _relayAgents = {_agent, _unowned};

class _DmAdds extends Fake implements ChannelActions {
  _DmAdds(this.landsIn, {this.unconfirmed = 0, this.existing = _twoPeople});

  final String landsIn;
  final Set<String> existing;

  /// How many times the DM the add lands in fails to confirm.
  int unconfirmed;
  final calls = <({List<String> pubkeys, DmHistory history})>[];
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
    required DmHistory history,
  }) async {
    expect(channelId, _dmId);
    calls.add((pubkeys: pubkeys, history: history));
    // The relay: a person joining a DM of two or more people needs a choice.
    final people = existing.where((key) => !_relayAgents.contains(key));
    if (history == DmHistory.ask &&
        people.length >= 2 &&
        pubkeys.any((key) => !_relayAgents.contains(key))) {
      throw const DmNeedsHistoryChoice();
    }
    _confirm();
    return _dm(landsIn);
  }
}

/// Opens the DM add sheet, picks [name], and taps Add.
Future<List<Channel>> _add(
  WidgetTester tester,
  _DmAdds actions,
  String name,
) async {
  final movedTo = <Channel>[];
  await tester.pumpWidget(
    ProviderScope(
      overrides: [
        channelActionsProvider.overrideWithValue(actions),
        channelMembersProvider(_dmId).overrideWith((ref) async => const []),
        agentOwnersProvider.overrideWith(
          (ref) async => {_agent: _person, _claimsAgent: _person},
        ),
        relaySessionProvider.overrideWith(
          () => RecipientDirectorySession(
            {
              _agent: 'Scout',
              _person: 'Robin',
              _unowned: 'Wren',
              _claimsAgent: 'Sly',
            },
            agents: {_agent, _unowned, _claimsAgent},
          ),
        ),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: AddChannelMembersSheet(
            channelId: _dmId,
            existingPubkeys: actions.existing,
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

const _ask = DmHistory.ask;

void main() {
  testWidgets('an agent joins the DM with its history without asking', (
    tester,
  ) async {
    final actions = _DmAdds(_dmId);
    final movedTo = await _add(tester, actions, 'Scout');

    expect(find.text('Include history'), findsNothing);
    expect(actions.calls.single.pubkeys, [_agent]);
    expect(actions.calls.single.history, _ask);
    expect(movedTo.single.id, _dmId);
  });

  testWidgets('an agent the relay knows joins without asking, owner or not', (
    tester,
  ) async {
    final actions = _DmAdds(_dmId);
    final movedTo = await _add(tester, actions, 'Wren');

    expect(find.text('Include history'), findsNothing);
    expect(actions.calls.single.history, _ask);
    expect(movedTo.single.id, _dmId);
  });

  testWidgets('someone who only claims to be an agent is asked about', (
    tester,
  ) async {
    final actions = _DmAdds(_dmId);
    await _add(tester, actions, 'Sly');

    expect(find.text('Add Sly?'), findsOneWidget);
    expect(actions.calls.single.history, _ask);
  });

  testWidgets('a member who only claims to be an agent counts as a person', (
    tester,
  ) async {
    // The viewer and someone the app would have taken for an agent.
    final actions = _DmAdds(_dmId, existing: {'self', _claimsAgent});
    await _add(tester, actions, 'Robin');

    expect(find.text('Add Robin?'), findsOneWidget);
  });

  testWidgets('adding a person asks, and Include history keeps this DM', (
    tester,
  ) async {
    final actions = _DmAdds(_dmId);
    final movedTo = await _add(tester, actions, 'Robin');

    expect(find.text('Add Robin?'), findsOneWidget);
    expect(find.text('Adding…'), findsNothing);
    expect(actions.calls.single.history, _ask);
    await tester.tap(find.text('Include history'));
    await tester.pumpAndSettle();

    expect(actions.calls.last.pubkeys, [_person]);
    expect(actions.calls.map((call) => call.history), [
      _ask,
      DmHistory.include,
    ]);
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

    expect(actions.calls.last.history, DmHistory.startNew);
    expect(movedTo.single.id, 'dm-2');
  });

  testWidgets('a person added to a DM of one person starts a new DM', (
    tester,
  ) async {
    final actions = _DmAdds('dm-2', existing: {'self', _agent});
    final movedTo = await _add(tester, actions, 'Robin');

    // The relay starts the new DM for a person without asking.
    expect(find.text('Include history'), findsNothing);
    expect(actions.calls.single.pubkeys, [_person]);
    expect(actions.calls.single.history, _ask);
    expect(movedTo.single.id, 'dm-2');
  });

  testWidgets('cancelling the choice adds no one', (tester) async {
    final actions = _DmAdds(_dmId);
    await _add(tester, actions, 'Robin');

    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    expect(actions.calls.single.history, _ask);
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

      final notice = find.text('Added. Tap Add again to open the DM.');
      expect(notice, findsOneWidget);
      // It says what to do next, not that something failed.
      final context = tester.element(notice);
      expect(
        tester.widget<Text>(notice).style?.color,
        Theme.of(context).colorScheme.onSurfaceVariant,
      );
      expect(movedTo, isEmpty);

      // Add only looks for the DM again, until it shows.
      for (var attempt = 0; attempt < 2; attempt++) {
        await tester.tap(
          find.byKey(const ValueKey('add-channel-members-submit')),
        );
        await tester.pumpAndSettle();
      }

      expect(actions.calls, hasLength(2));
      expect(actions.opens, ['dm-2', 'dm-2']);
      expect(movedTo.single.id, 'dm-2');
    },
  );
}

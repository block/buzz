import 'package:buzz/features/channels/add_members_sheet.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/recipient_directory_session.dart';

const _dmId = 'dm-1';
final _agent = 'f' * 64;
final _person = 'e' * 64;

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
  _DmAdds(this.landsIn);

  final String landsIn;
  final calls = <({List<String> pubkeys, bool includeHistory})>[];

  @override
  Future<Channel> addToDm({
    required String channelId,
    required List<String> pubkeys,
    required bool includeHistory,
  }) async {
    expect(channelId, _dmId);
    calls.add((pubkeys: pubkeys, includeHistory: includeHistory));
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
        relaySessionProvider.overrideWith(
          () => RecipientDirectorySession(
            {_agent: 'Scout', _person: 'Robin'},
            agents: {_agent},
          ),
        ),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: AddChannelMembersSheet(
            channelId: _dmId,
            existingPubkeys: const {},
            isDm: true,
            onMovedTo: movedTo.add,
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
    expect(movedTo, isEmpty);
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
    expect(movedTo, isEmpty);
  });

  testWidgets('Start new DM adds without history and opens the new DM', (
    tester,
  ) async {
    final actions = _DmAdds('dm-2');
    final movedTo = await _add(tester, actions, 'Robin');

    await tester.tap(find.text('Start new DM'));
    await tester.pumpAndSettle();

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
}

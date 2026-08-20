import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/members_sheet.dart';

import '../../helpers/widget_helpers.dart';

/// The add-member entry point is the only way, on mobile, to put a person or
/// agent into a channel they are not already in — the compose bar's `@`
/// autocomplete only offers agents that already share a channel with you.
void main() {
  Channel channel({String channelType = 'stream', DateTime? archivedAt}) =>
      Channel(
        id: 'chan-1',
        name: 'groceries',
        channelType: channelType,
        visibility: 'open',
        description: '',
        createdBy: 'someone',
        createdAt: DateTime.utc(2026),
        memberCount: 1,
        archivedAt: archivedAt,
        isMember: true,
      );

  Widget buildSheet(Channel target) => WidgetHelpers.testable(
    overrides: [
      channelMembersProvider(
        'chan-1',
      ).overrideWith((ref) async => const <ChannelMember>[]),
    ],
    child: MembersSheet(channel: target, currentPubkey: 'me'),
  );

  testWidgets('offers the add-member entry point on a regular channel', (
    tester,
  ) async {
    await tester.pumpWidget(buildSheet(channel()));
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('open-add-member')), findsOneWidget);
  });

  testWidgets('hides the add-member entry point on a DM', (tester) async {
    await tester.pumpWidget(buildSheet(channel(channelType: 'dm')));
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('open-add-member')), findsNothing);
  });

  testWidgets('hides the add-member entry point on an archived channel', (
    tester,
  ) async {
    await tester.pumpWidget(
      buildSheet(channel(archivedAt: DateTime.utc(2026, 2))),
    );
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('open-add-member')), findsNothing);
  });
}

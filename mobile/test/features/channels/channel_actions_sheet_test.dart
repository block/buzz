import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_actions_sheet.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _currentPubkey = 'me';

Channel _channel({String type = 'stream', bool isArchived = false}) => Channel(
  id: 'channel-id',
  name: type == 'dm' ? 'Alice' : 'general',
  channelType: type,
  visibility: 'open',
  description: '',
  createdBy: 'owner',
  createdAt: DateTime(2025),
  memberCount: 2,
  isMember: true,
  archivedAt: isArchived ? DateTime(2025, 1, 2) : null,
);

Widget _app({
  required Channel channel,
  required Future<List<ChannelMember>> Function() loadMembers,
  bool isUnread = false,
  AsyncValue<Map<String, String>> agentOwners = const AsyncValue.data(
    <String, String>{},
  ),
  ChannelActions Function(Ref ref)? createChannelActions,
  String? currentPubkey = _currentPubkey,
}) => ProviderScope(
  overrides: [
    currentPubkeyProvider.overrideWith((ref) => currentPubkey),
    channelMembersProvider(channel.id).overrideWith((ref) => loadMembers()),
    agentOwnersProvider.overrideWithValue(agentOwners),
    if (createChannelActions != null)
      channelActionsProvider.overrideWith(createChannelActions),
  ],
  child: MaterialApp(
    theme: AppTheme.light(),
    home: Scaffold(
      body: ChannelActionsSheet(channel: channel, isUnread: isUnread),
    ),
  ),
);

Widget _modalApp({
  required Channel channel,
  required Future<List<ChannelMember>> Function() loadMembers,
  required ChannelActions Function(Ref ref) createChannelActions,
}) => ProviderScope(
  overrides: [
    currentPubkeyProvider.overrideWith((ref) => _currentPubkey),
    channelMembersProvider(channel.id).overrideWith((ref) => loadMembers()),
    channelActionsProvider.overrideWith(createChannelActions),
  ],
  child: MaterialApp(
    theme: AppTheme.light(),
    home: Builder(
      builder: (context) => Scaffold(
        body: TextButton(
          onPressed: () => showChannelActionsSheet(
            context: context,
            channel: channel,
            isUnread: false,
          ),
          child: const Text('Open actions'),
        ),
      ),
    ),
  ),
);

void main() {
  testWidgets('owner sees the complete regular-channel action set', (
    tester,
  ) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    for (final label in [
      'Star',
      'Mark Unread',
      'Move to section…',
      'Mute channel',
      'Manage channel',
      'Copy channel name',
      'Copy channel ID',
      'Leave channel',
      'Archive channel',
      'Delete channel',
    ]) {
      expect(find.text(label), findsOneWidget, reason: label);
    }

    final moveTop = tester.getTopLeft(find.text('Move to section…')).dy;
    final muteTop = tester.getTopLeft(find.text('Mute channel')).dy;
    final manageTop = tester.getTopLeft(find.text('Manage channel')).dy;
    final copyNameTop = tester.getTopLeft(find.text('Copy channel name')).dy;
    final copyIdTop = tester.getTopLeft(find.text('Copy channel ID')).dy;
    expect(moveTop, lessThan(muteTop));
    expect(muteTop, lessThan(manageTop));
    expect(manageTop, lessThan(copyNameTop));
    expect(copyNameTop, lessThan(copyIdTop));
  });

  testWidgets('unread channel uses the Mark Read label', (tester) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        isUnread: true,
        loadMembers: () async => const [],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Mark Read'), findsOneWidget);
    expect(find.text('Mark Unread'), findsNothing);
  });

  testWidgets('admin can archive but cannot delete', (tester) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'admin',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsOneWidget);
    expect(find.text('Delete channel'), findsNothing);
  });

  testWidgets('verified owner agent grants archive and delete', (tester) async {
    const agentPubkey = 'agent';
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        agentOwners: const AsyncValue.data({agentPubkey: _currentPubkey}),
        loadMembers: () async => [
          ChannelMember(
            pubkey: agentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsOneWidget);
    expect(find.text('Delete channel'), findsOneWidget);
  });

  testWidgets('unresolved identity grants no lifecycle actions', (
    tester,
  ) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        currentPubkey: null,
        loadMembers: () async => [
          ChannelMember(
            pubkey: 'ordinary-owner',
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsNothing);
    expect(find.text('Delete channel'), findsNothing);
  });

  testWidgets('archived owner can unarchive but cannot delete', (tester) async {
    late _FakeChannelActions actions;
    await tester.pumpWidget(
      _app(
        channel: _channel(isArchived: true),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
        createChannelActions: (ref) => actions = _FakeChannelActions(ref),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsNothing);
    expect(find.text('Unarchive channel'), findsOneWidget);
    expect(find.text('Delete channel'), findsNothing);

    await tester.tap(find.text('Unarchive channel'));
    await tester.pumpAndSettle();
    expect(find.text('Unarchive #general?'), findsOneWidget);
    await tester.tap(find.widgetWithText(FilledButton, 'Unarchive'));
    await tester.pumpAndSettle();

    expect(actions.unarchivedChannelId, 'channel-id');
  });

  testWidgets('owned non-owner agent grants no lifecycle actions', (
    tester,
  ) async {
    const agentPubkey = 'agent';
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        agentOwners: const AsyncValue.data({agentPubkey: _currentPubkey}),
        loadMembers: () async => [
          ChannelMember(
            pubkey: agentPubkey,
            role: 'bot',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsNothing);
    expect(find.text('Delete channel'), findsNothing);
  });

  testWidgets('agent ownership loading keeps lifecycle actions pending', (
    tester,
  ) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        agentOwners: const AsyncValue.loading(),
        loadMembers: () async => const [],
      ),
    );
    await tester.pump();

    expect(find.text('Loading channel actions…'), findsOneWidget);
    expect(find.text('Archive channel'), findsNothing);
    expect(find.text('Delete channel'), findsNothing);
  });

  testWidgets('member sees neither owner action', (tester) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'member',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsNothing);
    expect(find.text('Delete channel'), findsNothing);
    expect(find.text('Leave channel'), findsOneWidget);
  });

  testWidgets('shows loading and unavailable capability states', (
    tester,
  ) async {
    final pending = Completer<List<ChannelMember>>();
    await tester.pumpWidget(
      _app(channel: _channel(), loadMembers: () => pending.future),
    );
    await tester.pump();
    expect(find.text('Loading channel actions…'), findsOneWidget);

    pending.completeError(Exception('relay unavailable'));
    await tester.pumpAndSettle();
    expect(find.text('Channel actions unavailable'), findsOneWidget);
  });

  testWidgets(
    'manage leave closes both nested sheets without popping the page',
    (tester) async {
      await tester.pumpWidget(
        _modalApp(
          channel: _channel(),
          loadMembers: () async => const [],
          createChannelActions: (ref) => _FakeChannelActions(ref),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Open actions'));
      await tester.pumpAndSettle();

      await tester.tap(find.text('Manage channel'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Leave channel').last);
      await tester.pumpAndSettle();

      expect(find.byType(ChannelActionsSheet), findsNothing);
      expect(find.byType(Scaffold), findsOneWidget);
    },
  );

  testWidgets('DM omits quick actions, then shows mute and copy rows', (
    tester,
  ) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(type: 'dm'),
        loadMembers: () async => const [],
      ),
    );
    await tester.pumpAndSettle();

    for (final label in [
      'Mute channel',
      'Copy channel name',
      'Copy channel ID',
    ]) {
      expect(find.text(label), findsOneWidget, reason: label);
    }
    for (final label in [
      'Star',
      'Unstar',
      'Mark Unread',
      'Mark Read',
      'Move to section…',
      'Manage channel',
      'Leave channel',
      'Archive channel',
      'Delete channel',
    ]) {
      expect(find.text(label), findsNothing, reason: label);
    }

    final muteTop = tester.getTopLeft(find.text('Mute channel')).dy;
    final copyNameTop = tester.getTopLeft(find.text('Copy channel name')).dy;
    final copyIdTop = tester.getTopLeft(find.text('Copy channel ID')).dy;
    expect(muteTop, lessThan(copyNameTop));
    expect(copyNameTop, lessThan(copyIdTop));
  });
}

class _FakeChannelActions extends ChannelActions {
  _FakeChannelActions(Ref ref)
    : super(
        ref: ref,
        session: ref.read(relaySessionProvider.notifier),
        signedEventRelay: SignedEventRelay(
          session: ref.read(relaySessionProvider.notifier),
          nsec: null,
        ),
        currentPubkey: _currentPubkey,
      );

  String? unarchivedChannelId;

  @override
  Future<void> leaveChannel(String channelId) async {}

  @override
  Future<void> unarchiveChannel(String channelId) async {
    unarchivedChannelId = channelId;
  }
}

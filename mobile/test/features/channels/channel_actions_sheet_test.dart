import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_actions_sheet.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _currentPubkey = 'me';

Channel _channel({String type = 'stream'}) => Channel(
  id: 'channel-id',
  name: type == 'dm' ? 'Alice' : 'general',
  channelType: type,
  visibility: 'open',
  description: '',
  createdBy: 'owner',
  createdAt: DateTime(2025),
  memberCount: 2,
  isMember: true,
);

Widget _app({
  required Channel channel,
  required Future<List<ChannelMember>> Function() loadMembers,
}) => ProviderScope(
  overrides: [
    currentPubkeyProvider.overrideWith((ref) => _currentPubkey),
    channelMembersProvider(channel.id).overrideWith((ref) => loadMembers()),
  ],
  child: MaterialApp(
    theme: AppTheme.light(),
    home: Scaffold(
      body: ChannelActionsSheet(channel: channel, isUnread: false),
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
      'Copy channel name',
      'Copy channel ID',
      'Move to section',
      'Mark as unread',
      'Mute channel',
      'Star channel',
      'Manage channel',
      'Leave channel',
      'Archive channel',
      'Delete channel',
    ]) {
      expect(find.text(label), findsOneWidget, reason: label);
    }
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

  testWidgets('DM keeps only copy, read, and mute actions', (tester) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(type: 'dm'),
        loadMembers: () async => const [],
      ),
    );
    await tester.pumpAndSettle();

    for (final label in [
      'Copy channel name',
      'Copy channel ID',
      'Mark as unread',
      'Mute channel',
    ]) {
      expect(find.text(label), findsOneWidget, reason: label);
    }
    for (final label in [
      'Move to section',
      'Star channel',
      'Manage channel',
      'Leave channel',
      'Archive channel',
      'Delete channel',
    ]) {
      expect(find.text(label), findsNothing, reason: label);
    }
  });
}

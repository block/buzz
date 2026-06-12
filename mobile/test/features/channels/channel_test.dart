import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/channel.dart';

void main() {
  group('Channel.fromJson', () {
    test('parses a full channel response', () {
      final json = {
        'id': 'abc-123',
        'name': 'general',
        'channel_type': 'stream',
        'visibility': 'open',
        'description': 'General discussion',
        'topic': 'Welcome!',
        'purpose': 'Team chat',
        'created_by': 'deadbeef',
        'created_at': '2025-01-01T00:00:00+00:00',
        'member_count': 42,
        'last_message_at': '2025-06-01T12:00:00+00:00',
        'archived_at': null,
        'participants': ['Alice', 'Bob'],
        'participant_pubkeys': ['alice', 'bob'],
        'is_member': true,
      };

      final channel = Channel.fromJson(json);

      expect(channel.id, 'abc-123');
      expect(channel.name, 'general');
      expect(channel.channelType, 'stream');
      expect(channel.visibility, 'open');
      expect(channel.description, 'General discussion');
      expect(channel.topic, 'Welcome!');
      expect(channel.purpose, 'Team chat');
      expect(channel.memberCount, 42);
      expect(channel.participants, ['Alice', 'Bob']);
      expect(channel.participantPubkeys, ['alice', 'bob']);
      expect(channel.isMember, isTrue);
      expect(channel.isStream, isTrue);
      expect(channel.isForum, isFalse);
      expect(channel.isDm, isFalse);
      expect(channel.isPrivate, isFalse);
    });

    test('handles null optional fields', () {
      final json = {
        'id': 'abc-123',
        'name': 'private-chat',
        'channel_type': 'stream',
        'visibility': 'private',
        'description': null,
        'topic': null,
        'purpose': null,
        'created_by': 'deadbeef',
        'created_at': '2025-01-01T00:00:00+00:00',
        'member_count': 2,
        'last_message_at': null,
        'archived_at': '2025-01-02T00:00:00+00:00',
        'is_member': false,
      };

      final channel = Channel.fromJson(json);

      expect(channel.description, '');
      expect(channel.topic, isNull);
      expect(channel.lastMessageAt, isNull);
      expect(channel.isArchived, isTrue);
      expect(channel.isMember, isFalse);
      expect(channel.isPrivate, isTrue);
    });

    test('defaults is_member to false when missing', () {
      final json = {
        'id': 'abc-123',
        'name': 'test',
        'channel_type': 'forum',
        'visibility': 'open',
        'created_by': 'deadbeef',
        'created_at': '2025-01-01T00:00:00+00:00',
        'member_count': 0,
      };

      final channel = Channel.fromJson(json);

      expect(channel.isMember, isFalse);
      expect(channel.isForum, isTrue);
    });
  });

  group('Channel.displayLabel', () {
    Channel makeDm({
      List<String> participants = const [],
      List<String> participantPubkeys = const [],
    }) => Channel(
      id: '1',
      name: 'dm-name',
      channelType: 'dm',
      visibility: 'open',
      description: '',
      createdBy: 'x',
      createdAt: DateTime(2025),
      memberCount: 2,
      participants: participants,
      participantPubkeys: participantPubkeys,
    );

    test('returns name for non-DM channels', () {
      final channel = Channel(
        id: '1',
        name: 'general',
        channelType: 'stream',
        visibility: 'open',
        description: '',
        createdBy: 'x',
        createdAt: DateTime(2025),
        memberCount: 1,
      );
      expect(channel.displayLabel(), 'general');
      expect(channel.displayLabel(currentPubkey: 'abc'), 'general');
    });

    test('returns name for DM with empty participants', () {
      final channel = makeDm();
      expect(channel.displayLabel(), 'dm-name');
    });

    test('returns all participants when no currentPubkey', () {
      final channel = makeDm(
        participants: ['Alice', 'Bob'],
        participantPubkeys: ['aaa', 'bbb'],
      );
      expect(channel.displayLabel(), 'Alice, Bob');
    });

    test('filters out current user by pubkey', () {
      final channel = makeDm(
        participants: ['You', 'Alice'],
        participantPubkeys: ['self', 'alice'],
      );
      expect(channel.displayLabel(currentPubkey: 'SELF'), 'Alice');
    });

    test('falls back to all participants when self is only participant', () {
      final channel = makeDm(
        participants: ['You'],
        participantPubkeys: ['self'],
      );
      expect(channel.displayLabel(currentPubkey: 'self'), 'You');
    });

    test('handles mismatched participants and pubkeys lengths', () {
      final channel = makeDm(
        participants: ['Alice', 'Bob', 'Carol'],
        participantPubkeys: ['alice'],
      );
      // Only index 0 has a pubkey; indexes 1-2 have no pubkey to match,
      // so they always appear. Filtering out 'alice' leaves Bob and Carol.
      expect(channel.displayLabel(currentPubkey: 'alice'), 'Bob, Carol');
    });
  });

  group('Channel.copyWith', () {
    final base = Channel(
      id: '1',
      name: 'test',
      channelType: 'stream',
      visibility: 'open',
      description: '',
      createdBy: 'x',
      createdAt: DateTime(2025),
      memberCount: 5,
      archivedAt: DateTime(2025, 1, 2),
      lastMessageAt: DateTime(2025, 6, 1),
      isMember: true,
    );

    test('can explicitly null out archivedAt', () {
      final updated = base.copyWith(archivedAt: null);
      expect(updated.archivedAt, isNull);
    });

    test('can explicitly null out lastMessageAt', () {
      final updated = base.copyWith(lastMessageAt: null);
      expect(updated.lastMessageAt, isNull);
    });

    test('preserves archivedAt when not specified', () {
      final updated = base.copyWith(memberCount: 10);
      expect(updated.archivedAt, base.archivedAt);
      expect(updated.memberCount, 10);
    });

    test('can set new archivedAt value', () {
      final newDate = DateTime(2026);
      final updated = base.copyWith(archivedAt: newDate);
      expect(updated.archivedAt, newDate);
    });
  });
}

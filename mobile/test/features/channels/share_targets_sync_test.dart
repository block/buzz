import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/share_targets_sync.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:flutter_test/flutter_test.dart';

Channel _channel({
  required String id,
  required String name,
  String channelType = 'stream',
  bool isMember = true,
  DateTime? archivedAt,
  DateTime? lastMessageAt,
  List<String> participants = const [],
  List<String> participantPubkeys = const [],
}) => Channel(
  id: id,
  name: name,
  channelType: channelType,
  visibility: 'open',
  description: '',
  createdBy: 'abc123',
  createdAt: DateTime(2025),
  memberCount: 3,
  isMember: isMember,
  archivedAt: archivedAt,
  lastMessageAt: lastMessageAt,
  participants: participants,
  participantPubkeys: participantPubkeys,
);

Community _community(String id, String name) => Community(
  id: id,
  name: name,
  relayUrl: 'wss://$id.example',
  addedAt: DateTime.utc(2026),
);

void main() {
  test('publishes member channels newest first with DM labels resolved', () {
    final snapshot = buildShareTargetsSnapshot(
      community: _community('c1', 'Tecnocracia'),
      communities: [_community('c1', 'Tecnocracia'), _community('c2', 'Other')],
      channels: [
        _channel(id: 'old', name: 'old', lastMessageAt: DateTime.utc(2026, 1)),
        _channel(id: 'new', name: 'new', lastMessageAt: DateTime.utc(2026, 5)),
        _channel(
          id: 'dm',
          name: '',
          channelType: 'dm',
          participants: ['Me', 'Ana'],
          participantPubkeys: ['me', 'ana'],
          lastMessageAt: DateTime.utc(2026, 3),
        ),
        _channel(id: 'left', name: 'left', isMember: false),
        _channel(id: 'gone', name: 'gone', archivedAt: DateTime.utc(2026)),
      ],
      currentPubkey: 'me',
    );

    expect(snapshot.communityId, 'c1');
    expect(snapshot.communities.map((c) => c.name), ['Tecnocracia', 'Other']);
    expect(snapshot.targets.map((t) => t.channelId), ['new', 'dm', 'old']);
    expect(snapshot.targets[1].label, 'Ana');
    expect(snapshot.targets[1].isDm, isTrue);
    expect(snapshot.toArguments()['targets'], hasLength(3));
  });

  test('fingerprint changes with labels and activity', () {
    ShareTargetsSnapshotFingerprint fingerprint(DateTime when) =>
        buildShareTargetsSnapshot(
          community: _community('c1', 'T'),
          communities: [_community('c1', 'T')],
          channels: [_channel(id: 'a', name: 'a', lastMessageAt: when)],
          currentPubkey: null,
        ).fingerprint;
    expect(
      fingerprint(DateTime.utc(2026, 1)),
      fingerprint(DateTime.utc(2026, 1)),
    );
    expect(
      fingerprint(DateTime.utc(2026, 1)),
      isNot(fingerprint(DateTime.utc(2026, 2))),
    );
  });
}

typedef ShareTargetsSnapshotFingerprint = String;

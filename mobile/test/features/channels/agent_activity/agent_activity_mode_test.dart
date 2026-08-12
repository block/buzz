import 'package:buzz/features/channels/agent_activity/agent_activity_mode.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  const me = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
  const other =
      'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

  AgentActivityMode select({
    String? ownerPubkey = other,
    String? myPubkey = me,
    String? channelType = 'stream',
    bool isCurrentMember = true,
  }) => selectAgentActivityMode(
    ownerPubkey: ownerPubkey,
    myPubkey: myPubkey,
    channelType: channelType,
    isCurrentMember: isCurrentMember,
  );

  test('unresolved ownership fails safely to eligible shared mode', () {
    expect(select(ownerPubkey: null), AgentActivityMode.shared);
    expect(select(myPubkey: null), AgentActivityMode.shared);
  });

  test('malformed or nonmatching ownership cannot select owner mode', () {
    for (final owner in ['not-a-key', 'A' * 64, 'a' * 63, 'a' * 65]) {
      expect(
        select(ownerPubkey: owner, myPubkey: owner),
        AgentActivityMode.shared,
        reason: owner,
      );
    }
    expect(select(ownerPubkey: other), AgentActivityMode.shared);
  });

  test('only exact verified owner equality selects the full owner mode', () {
    expect(select(ownerPubkey: me), AgentActivityMode.owner);
    expect(
      select(ownerPubkey: me, myPubkey: 'A' * 64),
      AgentActivityMode.shared,
    );
    expect(
      select(ownerPubkey: me, channelType: 'dm', isCurrentMember: false),
      AgentActivityMode.owner,
      reason: 'the existing owner-only path is independent of shared access',
    );
  });

  test('shared mode is limited to current stream and forum members', () {
    expect(select(channelType: 'stream'), AgentActivityMode.shared);
    expect(select(channelType: 'forum'), AgentActivityMode.shared);
    expect(select(channelType: 'dm'), AgentActivityMode.unavailable);
    expect(select(channelType: 'unknown'), AgentActivityMode.unavailable);
    expect(select(channelType: null), AgentActivityMode.unavailable);
    expect(select(isCurrentMember: false), AgentActivityMode.unavailable);
    expect(
      select(ownerPubkey: null, isCurrentMember: false),
      AgentActivityMode.unavailable,
    );
  });
}

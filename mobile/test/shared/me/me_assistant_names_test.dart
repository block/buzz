import 'package:buzz/shared/identity_names/identity_names.dart';
import 'package:buzz/shared/me/me_assistant_names.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  final atlas = 'a' * 64;
  final uncached = 'b' * 64;
  final agents = [
    AgentDirectoryEntry(pubkey: atlas, displayName: 'Atlas'),
    AgentDirectoryEntry(pubkey: uncached, displayName: 'Beacon'),
  ];

  test('Solo prefers the inventory name to a cached profile name', () {
    final cached = IdentityNameSources(
      profiles: {atlas: UserProfile(pubkey: atlas, displayName: 'Old name')},
    );
    final names = meAssistantNames(cached, agents);
    expect(names.labelFor(atlas), 'Atlas');
    expect(names.labelFor(uncached), 'Beacon');
    // An uncached profile stays absent, so the caller still loads it.
    expect(meAssistantNameSources(cached, agents).profiles, contains(atlas));
    expect(
      meAssistantNameSources(cached, agents).profiles,
      isNot(contains(uncached)),
    );
  });

  test('a verified name still wins over the inventory name', () {
    final verified = IdentityNameSources(
      profiles: {atlas: UserProfile(pubkey: atlas, displayName: 'Old name')},
      verifiedNames: {atlas: 'Atlas Prime'},
    );
    expect(meAssistantNames(verified, agents).labelFor(atlas), 'Atlas Prime');
  });
}

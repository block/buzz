enum AgentActivityMode { owner, shared, unavailable }

/// Selects the owner-only transcript only for an exact verified owner match.
///
/// Everyone else receives the sanitized stream only when they are a current
/// member of a non-DM shared channel. Unresolved ownership therefore fails away
/// from the privileged path without widening shared eligibility.
AgentActivityMode selectAgentActivityMode({
  required String? ownerPubkey,
  required String? myPubkey,
  required String? channelType,
  required bool isCurrentMember,
}) {
  if (ownerPubkey != null &&
      myPubkey != null &&
      _lowercaseHexPubkey.hasMatch(ownerPubkey) &&
      _lowercaseHexPubkey.hasMatch(myPubkey) &&
      ownerPubkey == myPubkey) {
    return AgentActivityMode.owner;
  }

  if (!isCurrentMember || (channelType != 'stream' && channelType != 'forum')) {
    return AgentActivityMode.unavailable;
  }
  return AgentActivityMode.shared;
}

final _lowercaseHexPubkey = RegExp(r'^[0-9a-f]{64}$');

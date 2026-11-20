part of 'deep_link.dart';

/// `buzz://agent/new?card=<id>`: open an editable agent draft for a published
/// Builderlab agent card. The QR on a card leads to its share page, whose
/// "Add to Buzz" button opens this link. Opening it never creates an agent.
///
/// The link carries only the card id. The draft itself comes from the card's
/// `agent.json` on this build's avatars CDN, so instructions can be any length
/// while the printed QR stays small.
class AgentCreationDeepLink extends BuzzDeepLink {
  /// Creates a link for the published card [cardId].
  const AgentCreationDeepLink(this.cardId);

  /// Published card id, opaque to the app (today `av_…`).
  final String cardId;

  @override
  bool operator ==(Object other) =>
      other is AgentCreationDeepLink && cardId == other.cardId;

  @override
  int get hashCode => cardId.hashCode;

  @override
  String toString() => 'AgentCreationDeepLink($cardId)';
}

/// Parses `buzz://agent/new?card=<id>`; anything else is not this link.
AgentCreationDeepLink? parseAgentCreationDeepLink(Uri uri) {
  if (uri.scheme != 'buzz' ||
      uri.host != 'agent' ||
      uri.path != '/new' ||
      uri.hasPort ||
      uri.userInfo.isNotEmpty ||
      uri.hasFragment) {
    return null;
  }
  final params = uri.queryParametersAll;
  final card = params['card'];
  if (params.length != 1 || card == null || card.length != 1) return null;
  // The id is opaque, but it becomes one path segment of the card's URL.
  if (const {'', '.', '..'}.contains(card.single)) return null;
  return AgentCreationDeepLink(card.single);
}

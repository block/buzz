import '../deeplink/community_link_input.dart';
import '../deeplink/deep_link.dart';

/// A saved account destination must also be a supported mobile address. Keep
/// explicit ports and reject paths, credentials, and insecure transports.
String? knownCommunityAddress(String value) {
  final uri = Uri.tryParse(value);
  if (uri == null || !{'wss', 'https'}.contains(uri.scheme)) return null;
  if (RegExp(r':0+/?$').hasMatch(value) ||
      (uri.hasPort && (uri.port < 1 || uri.port > 65535))) {
    return null;
  }
  final link = parseCommunityLinkInput(value);
  if (link == null) return null;
  final parsed = parseAddCommunityDeepLink(link);
  if (parsed == null) return null;
  final relay = Uri.parse(parsed.relayUrl);
  if (relay.hasPort && (relay.port < 1 || relay.port > 65535)) return null;
  return Uri(
    scheme: 'wss',
    host: relay.host.toLowerCase(),
    port: relay.hasPort && relay.port != 443 ? relay.port : null,
  ).toString();
}

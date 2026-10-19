import 'deep_link.dart';

/// Converts user-entered community addresses or invites to validated app links.
/// Other deep-link actions are deliberately excluded from this input.
Uri? parseCommunityLinkInput(String input) {
  final raw = input.trim();
  if (raw.isEmpty || raw.length > 8192 || RegExp(r'\s').hasMatch(raw)) {
    return null;
  }
  try {
    final uri = Uri.parse(raw.contains('://') ? raw : 'https://$raw');
    if (parseInviteDeepLink(uri) != null ||
        parseAddCommunityDeepLink(uri) != null) {
      return uri;
    }
    if (!{'https', 'wss'}.contains(uri.scheme) ||
        uri.host.isEmpty ||
        uri.userInfo.isNotEmpty ||
        uri.hasQuery ||
        uri.hasFragment ||
        (uri.path.isNotEmpty && uri.path != '/')) {
      return null;
    }
    final link = Uri(
      scheme: 'buzz',
      host: 'add-community',
      queryParameters: {
        'relay': uri.replace(scheme: 'wss', path: '').toString(),
        'name': uri.host,
      },
    );
    return parseAddCommunityDeepLink(link) == null ? null : link;
  } on FormatException {
    return null;
  }
}

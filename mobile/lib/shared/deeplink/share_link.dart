part of 'deep_link.dart';

/// `<scheme>://share?id=<uuid>`: the iOS share extension left items in the
/// app group under this id and opened the app to pick them up.
///
/// The extension opens a scheme equal to its own build's bundle identifier,
/// so a share always returns to the build it came from. Every build also
/// registers `buzz://`, which other links keep using.
class ShareDeepLink extends BuzzDeepLink {
  /// Creates a link for the pending share [shareId].
  const ShareDeepLink(this.shareId);

  /// Directory name of the pending share inside the app group.
  final String shareId;

  @override
  bool operator ==(Object other) =>
      other is ShareDeepLink && shareId == other.shareId;

  @override
  int get hashCode => shareId.hashCode;

  @override
  String toString() => 'ShareDeepLink($shareId)';
}

final _shareIdPattern = RegExp(
  r'^[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}$',
);

/// Parses a share hand-off; anything else is not this link.
///
/// The id must be the extension's uppercase UUID, so it can only name a
/// directory directly inside the pending-shares folder.
ShareDeepLink? parseShareDeepLink(Uri uri) {
  if (uri.scheme == 'http' || uri.scheme == 'https' || uri.host != 'share') {
    return null;
  }
  if (uri.path.isNotEmpty && uri.path != '/') return null;
  final id = uri.queryParameters['id'];
  if (id == null || !_shareIdPattern.hasMatch(id)) return null;
  return ShareDeepLink(id);
}

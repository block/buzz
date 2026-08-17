// Animated avatar URL scheme shared with the desktop client.
//
// The static poster is the normal selected frame. The animated PNG URL lives
// in the fragment so clients that do not understand the scheme render only
// the poster:
//
//   posterUrl#buzz-anim=encodedAnimationUrl
const _animatedAvatarSeparator = '#buzz-anim=';

/// The static poster and animated image URLs encoded in an avatar URL.
class AnimatedAvatarDescriptor {
  const AnimatedAvatarDescriptor({
    required this.posterUrl,
    required this.animationUrl,
  });

  final String posterUrl;
  final String animationUrl;
}

/// Parses the Buzz animated-avatar fragment scheme from [url].
///
/// Returns `null` when the poster or animation URL is missing, malformed, or
/// does not use HTTP(S).
AnimatedAvatarDescriptor? parseAnimatedAvatarUrl(String? url) {
  if (url == null || url.isEmpty) return null;

  final separatorIndex = url.indexOf(_animatedAvatarSeparator);
  if (separatorIndex <= 0) return null;

  final posterUrl = url.substring(0, separatorIndex);
  final encodedAnimationUrl = url.substring(
    separatorIndex + _animatedAvatarSeparator.length,
  );
  if (encodedAnimationUrl.isEmpty) return null;

  final String animationUrl;
  try {
    animationUrl = Uri.decodeComponent(encodedAnimationUrl);
  } on ArgumentError {
    return null;
  } on FormatException {
    return null;
  }

  if (!_isHttpUrl(posterUrl) || !_isHttpUrl(animationUrl)) return null;
  return AnimatedAvatarDescriptor(
    posterUrl: posterUrl,
    animationUrl: animationUrl,
  );
}

bool _isHttpUrl(String value) {
  final uri = Uri.tryParse(value);
  return uri != null &&
      (uri.scheme == 'http' || uri.scheme == 'https') &&
      uri.host.isNotEmpty;
}

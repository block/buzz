import 'dart:convert';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'community.dart';
import 'community_icon_provider.dart';

class CommunityProfile {
  const CommunityProfile(this.name);
  final String? name;
}

/// Null means an older relay. Failed reads throw so cached truth survives.
final communityProfileFetcherProvider =
    Provider<Future<CommunityProfile?> Function(String)>((ref) {
      final client = ref.watch(communityIconHttpClientProvider);
      return (relayUrl) async {
        final relay = Uri.parse(relayUrl);
        final uri = relay.replace(
          scheme: switch (relay.scheme) {
            'wss' => 'https',
            'ws' => 'http',
            _ => relay.scheme,
          },
        );
        final response = await client
            .get(uri, headers: const {'Accept': 'application/nostr+json'})
            .timeout(const Duration(seconds: 5));
        if (response.statusCode < 200 || response.statusCode >= 300) {
          throw StateError('Community information unavailable');
        }
        final document = jsonDecode(response.body);
        if (document is! Map<String, dynamic>) {
          throw const FormatException('Invalid relay information');
        }
        final profile = document['community_profile'];
        if (profile == null) return null;
        if (profile is! Map<String, dynamic> || !profile.containsKey('name')) {
          throw const FormatException('Invalid community profile');
        }
        final name = profile['name'];
        if (name != null &&
            (name is! String ||
                name.trim().isEmpty ||
                utf8.encode(name).length > 256)) {
          throw const FormatException('Invalid community name');
        }
        return CommunityProfile((name as String?)?.trim());
      };
    });

Community reconcileCommunityName(
  Community community,
  CommunityProfile profile,
) {
  final fallback = community.fallbackName ?? community.name;
  var nickname = community.localName?.trim();
  if (nickname == null && community.fallbackName == null) {
    final host = Uri.tryParse(community.relayUrl)?.host ?? '';
    final legacyIpLabel =
        (RegExp(r'^\d+(?:\.\d+){3}$').hasMatch(host) || host.contains(':')) &&
        community.name == host.split('.').first;
    nickname =
        community.name == Community.nameFromUrl(community.relayUrl) ||
            legacyIpLabel
        ? ''
        : community.name;
  }
  final display = (nickname != null && nickname.isNotEmpty)
      ? nickname
      : (profile.name ?? fallback);
  if (community.name == display &&
      community.canonicalName == profile.name &&
      community.localName == nickname &&
      community.fallbackName == fallback) {
    return community;
  }
  return community.copyWith(
    name: display,
    canonicalName: profile.name,
    localName: nickname,
    fallbackName: fallback,
  );
}

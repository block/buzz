import 'package:flutter/foundation.dart';

@immutable
class UserProfile {
  final String pubkey;
  final String? displayName;
  final String? avatarUrl;
  final String? about;
  final String? nip05Handle;

  /// NIP-OA verified owner pubkey from the profile's `auth` tag; non-null
  /// means this identity is an agent (mirrors desktop's `ownerPubkey`).
  final String? ownerPubkey;

  const UserProfile({
    required this.pubkey,
    this.displayName,
    this.avatarUrl,
    this.about,
    this.nip05Handle,
    this.ownerPubkey,
  });

  factory UserProfile.fromJson(Map<String, dynamic> json) => UserProfile(
    pubkey: json['pubkey'] as String,
    displayName: json['display_name'] as String?,
    avatarUrl: json['avatar_url'] as String?,
    about: json['about'] as String?,
    nip05Handle: json['nip05_handle'] as String?,
  );

  /// Short label: display name, or first 8 chars of pubkey.
  String get label =>
      displayName ??
      '${pubkey.length >= 8 ? pubkey.substring(0, 8) : pubkey}...';

  /// First letter for fallback avatar.
  String get initial =>
      (displayName?.isNotEmpty == true ? displayName! : pubkey)[0]
          .toUpperCase();
}

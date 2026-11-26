import 'package:flutter/foundation.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../../shared/crypto/nip44.dart';
import '../sidebar_flag_sync.dart';
import 'channel_stars_storage.dart';

class ChannelStarsCrypto {
  final Uint8List _conversationKey;

  ChannelStarsCrypto(String nsec, String pubkey)
    : _conversationKey = _deriveKey(nsec, pubkey);

  static Uint8List _deriveKey(String nsec, String pubkey) {
    final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    return getConversationKey(privkeyHex, pubkey);
  }

  String encrypt(String plaintext) => nip44Encrypt(_conversationKey, plaintext);

  String decrypt(String ciphertext) =>
      nip44Decrypt(_conversationKey, ciphertext);
}

/// Syncs the account's channel stars.
class ChannelStarsManager extends SidebarFlagSync<ChannelStarEntry> {
  ChannelStarsManager({
    ChannelFlagKind kind = ChannelFlagKind.stars,
    required super.pubkey,
    required super.relayUrl,
    required super.prefs,
    required ChannelStarsCrypto crypto,
    required super.relaySession,
    required super.signedEventRelay,
    required super.remoteEnabled,
    required super.onChanged,
    @visibleForTesting super.startupRetryBaseDelay = const Duration(seconds: 2),
  }) : super(
         coordinate: kind.coordinate,
         cacheKey: channelStarsKey(pubkey, relayUrl, kind),
         cachedFlags: ChannelStarsStorage(
           prefs,
         ).read(pubkey, relayUrl, kind).channels,
         decode: (json, strict) =>
             ChannelStarStore.fromJson(json, strict: strict).channels,
         encode: (flags) => ChannelStarStore(channels: flags).toJson(),
         entry: (enabled, timestamp) =>
             ChannelStarEntry(starred: enabled, updatedAt: timestamp),
         decrypt: crypto.decrypt,
         encrypt: crypto.encrypt,
       );

  ChannelStarStore get store => ChannelStarStore(channels: values);

  void starChannel(String channelId) => setFlag(channelId, true);

  void unstarChannel(String channelId) =>
      setFlag(channelId, false, skipAbsentClear: true);
}

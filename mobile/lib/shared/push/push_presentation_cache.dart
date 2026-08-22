import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../relay/nostr_models.dart';
import 'push_capability.dart';

const _pushPresentationChannel = MethodChannel('buzz/push');
// Keep these bridge payload bounds aligned with BuzzPushPresentationCacheStore.
const _maximumPresentationProfiles = 256;
const _maximumPresentationChannels = 512;
const _maximumAvatarSourceBytes = 512 * 1024;
const _maximumAvatarPNGBytes = 64 * 1024;
Future<void> _avatarEncodeTail = Future.value();

/// The latest best-effort App Group presentation-cache failure.
final pushPresentationCacheError = ValueNotifier<String?>(null);

/// Revalidates a relay event before it crosses into the native cache writer.
bool isVerifiedPushPresentationEvent(NostrEvent event) {
  try {
    nostr.Event(
      event.id,
      event.pubkey,
      event.createdAt,
      event.kind,
      event.tags,
      event.content,
      event.sig,
    );
    return true;
  } catch (_) {
    return false;
  }
}

/// Exports raw verified kind-0 events. Native code verifies them again before storage.
Future<void> cacheBuzzPushProfileEvents(
  String communityID,
  Iterable<NostrEvent> events,
) async {
  if (!buzzPushCapabilityEnabled ||
      defaultTargetPlatform != TargetPlatform.iOS ||
      communityID.isEmpty) {
    return;
  }
  final verified = _boundedNewestEvents(
    events,
    kind: 0,
    maximum: _maximumPresentationProfiles,
    scope: (event) => event.pubkey.toLowerCase(),
  ).values.toList();
  if (verified.isEmpty) return;
  await _invokeBestEffort('cachePresentationProfiles', {
    'communityId': communityID,
    'events': [for (final event in verified) event.toJson()],
  });
}

/// Exports verified channel metadata and membership for native authority checks.
Future<void> cacheBuzzPushChannelEvents(
  String communityID,
  Iterable<NostrEvent> metadataEvents,
  Iterable<NostrEvent> membershipEvents,
) async {
  if (!buzzPushCapabilityEnabled ||
      defaultTargetPlatform != TargetPlatform.iOS ||
      communityID.isEmpty) {
    return;
  }
  final batch = selectBoundedPushChannelEvents(
    metadataEvents,
    membershipEvents,
  );
  final verifiedMetadata = batch.metadata;
  final verifiedMembership = batch.membership;
  if (verifiedMetadata.isEmpty && verifiedMembership.isEmpty) return;
  await _invokeBestEffort('cachePresentationChannels', {
    'communityId': communityID,
    'metadataEvents': [for (final event in verifiedMetadata) event.toJson()],
    'membershipEvents': [
      for (final event in verifiedMembership) event.toJson(),
    ],
  });
}

/// Selects a bounded, paired set of verified channel metadata and membership events.
@visibleForTesting
({List<NostrEvent> metadata, List<NostrEvent> membership})
selectBoundedPushChannelEvents(
  Iterable<NostrEvent> metadataEvents,
  Iterable<NostrEvent> membershipEvents, {
  @visibleForTesting int maximumChannels = _maximumPresentationChannels,
}) {
  final verifiedMembershipByChannel = _boundedNewestEvents(
    membershipEvents,
    kind: 39002,
    maximum: maximumChannels,
    scope: (event) => event.getTagValue('d'),
  );
  final selectedChannelIDs = verifiedMembershipByChannel.keys.toSet();
  final verifiedMetadataByChannel = _boundedNewestEvents(
    metadataEvents,
    kind: 39000,
    maximum: maximumChannels,
    scope: (event) => event.getTagValue('d'),
    allowedScopes: selectedChannelIDs.isEmpty ? null : selectedChannelIDs,
  );
  if (selectedChannelIDs.isEmpty) {
    selectedChannelIDs.addAll(verifiedMetadataByChannel.keys);
  }
  final verifiedMetadata = [
    for (final entry in verifiedMetadataByChannel.entries)
      if (selectedChannelIDs.contains(entry.key)) entry.value,
  ];
  final verifiedMembership = [
    for (final entry in verifiedMembershipByChannel.entries)
      if (selectedChannelIDs.contains(entry.key)) entry.value,
  ];
  return (metadata: verifiedMetadata, membership: verifiedMembership);
}

Map<String, NostrEvent> _boundedNewestEvents(
  Iterable<NostrEvent> events, {
  required int kind,
  required int maximum,
  required String? Function(NostrEvent event) scope,
  Set<String>? allowedScopes,
}) {
  final selected = <String, NostrEvent>{};
  if (maximum <= 0) return selected;
  for (final event in events) {
    if (event.kind != kind || !isVerifiedPushPresentationEvent(event)) continue;
    final key = scope(event);
    if (key == null || key.isEmpty) continue;
    if (allowedScopes != null && !allowedScopes.contains(key)) continue;
    final existing = selected[key];
    if (existing != null) {
      if (_isNewerEvent(event, existing)) selected[key] = event;
      continue;
    }
    if (selected.length < maximum) {
      selected[key] = event;
      continue;
    }
    final oldest = selected.entries.reduce(
      (left, right) => _isNewerEvent(left.value, right.value) ? right : left,
    );
    if (_isNewerEvent(event, oldest.value)) {
      selected
        ..remove(oldest.key)
        ..[key] = event;
    }
  }
  return selected;
}

bool _isNewerEvent(NostrEvent candidate, NostrEvent existing) =>
    candidate.createdAt > existing.createdAt ||
    (candidate.createdAt == existing.createdAt &&
        candidate.id.compareTo(existing.id) < 0);

/// Reuses bytes already fetched for a visible foreground avatar.
///
/// This never starts network I/O. Oversized, malformed, or unsupported images
/// are ignored, and notification delivery remains independent of the cache.
Future<void> cacheBuzzPushAvatarFromLoadedBytes(
  String communityID,
  String sourceURL,
  Uint8List sourceBytes,
) async {
  if (!buzzPushCapabilityEnabled ||
      defaultTargetPlatform != TargetPlatform.iOS ||
      communityID.isEmpty ||
      sourceBytes.isEmpty ||
      sourceBytes.length > _maximumAvatarSourceBytes ||
      !_isRemoteImageURL(sourceURL)) {
    return;
  }
  final previous = _avatarEncodeTail;
  final release = Completer<void>();
  _avatarEncodeTail = release.future;
  await previous;
  try {
    final png = await _boundedAvatarPNG(sourceBytes);
    if (png == null) return;
    await _invokeBestEffort('cachePresentationAvatar', {
      'communityId': communityID,
      'sourceUrl': sourceURL,
      'png': png,
    });
  } finally {
    release.complete();
  }
}

Future<void> _invokeBestEffort(
  String method,
  Map<String, Object> arguments,
) async {
  try {
    await _pushPresentationChannel.invokeMethod<void>(method, arguments);
    pushPresentationCacheError.value = null;
  } on MissingPluginException {
    // Push-free builds and non-Runner embeddings intentionally omit the bridge.
  } catch (error, stackTrace) {
    pushPresentationCacheError.value = error.toString();
    debugPrint('Push presentation cache update failed: $error');
    debugPrintStack(stackTrace: stackTrace);
  }
}

bool _isRemoteImageURL(String value) {
  final uri = Uri.tryParse(value.trim());
  return uri != null &&
      (uri.scheme == 'http' || uri.scheme == 'https') &&
      uri.host.isNotEmpty &&
      uri.userInfo.isEmpty;
}

Future<Uint8List?> _boundedAvatarPNG(Uint8List sourceBytes) async {
  for (final size in const [128, 96, 64, 48]) {
    ui.Codec? codec;
    ui.Image? image;
    try {
      codec = await ui.instantiateImageCodec(
        sourceBytes,
        targetWidth: size,
        targetHeight: size,
        allowUpscaling: false,
      );
      final frame = await codec.getNextFrame();
      image = frame.image;
      final data = await image.toByteData(format: ui.ImageByteFormat.png);
      if (data == null) continue;
      final png = data.buffer.asUint8List(
        data.offsetInBytes,
        data.lengthInBytes,
      );
      if (png.isNotEmpty && png.length <= _maximumAvatarPNGBytes) return png;
    } catch (_) {
      return null;
    } finally {
      image?.dispose();
      codec?.dispose();
    }
  }
  return null;
}

import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../relay/nostr_models.dart';

const _pushPresentationChannel = MethodChannel('buzz/push');
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
  if (defaultTargetPlatform != TargetPlatform.iOS || communityID.isEmpty) {
    return;
  }
  final verified = _newestVerifiedEvents(
    events,
    kind: 0,
    scope: (event) => event.pubkey.toLowerCase(),
  ).values.toList();
  if (verified.isEmpty) return;
  await _invokeBestEffort({
    'section': 'profiles',
    'communityId': communityID,
    'events': [for (final event in verified) event.toJson()],
  });
}

/// Exports verified channel metadata and membership for native authority checks.
Future<void> cacheBuzzPushChannelEvents(
  String? communityID,
  Iterable<NostrEvent> metadataEvents,
  Iterable<NostrEvent> membershipEvents,
) async {
  if (defaultTargetPlatform != TargetPlatform.iOS ||
      communityID == null ||
      communityID.isEmpty) {
    return;
  }
  final batch = selectPushChannelEvents(metadataEvents, membershipEvents);
  final verifiedMetadata = batch.metadata;
  final verifiedMembership = batch.membership;
  if (verifiedMetadata.isEmpty && verifiedMembership.isEmpty) return;
  await _invokeBestEffort({
    'section': 'channels',
    'communityId': communityID,
    'metadataEvents': [for (final event in verifiedMetadata) event.toJson()],
    'membershipEvents': [
      for (final event in verifiedMembership) event.toJson(),
    ],
  });
}

/// Selects the newest paired verified channel metadata and membership events.
@visibleForTesting
({List<NostrEvent> metadata, List<NostrEvent> membership})
selectPushChannelEvents(
  Iterable<NostrEvent> metadataEvents,
  Iterable<NostrEvent> membershipEvents,
) {
  final verifiedMembershipByChannel = _newestVerifiedEvents(
    membershipEvents,
    kind: 39002,
    scope: (event) => event.getTagValue('d'),
  );
  final selectedChannelIDs = verifiedMembershipByChannel.keys.toSet();
  final verifiedMetadataByChannel = _newestVerifiedEvents(
    metadataEvents,
    kind: 39000,
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

Map<String, NostrEvent> _newestVerifiedEvents(
  Iterable<NostrEvent> events, {
  required int kind,
  required String? Function(NostrEvent event) scope,
  Set<String>? allowedScopes,
}) {
  final selected = <String, NostrEvent>{};
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
    selected[key] = event;
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
  if (defaultTargetPlatform != TargetPlatform.iOS ||
      communityID.isEmpty ||
      sourceBytes.isEmpty ||
      sourceBytes.length > _maximumAvatarSourceBytes ||
      !isCacheablePushAvatarSource(sourceURL)) {
    return;
  }
  final previous = _avatarEncodeTail;
  final release = Completer<void>();
  _avatarEncodeTail = release.future;
  await previous;
  try {
    final png = await _boundedAvatarPNG(sourceBytes);
    if (png == null) return;
    await _invokeBestEffort({
      'section': 'avatar',
      'communityId': communityID,
      'sourceUrl': sourceURL,
      'png': png,
    });
  } finally {
    release.complete();
  }
}

Future<void> _invokeBestEffort(Map<String, Object> arguments) async {
  try {
    await _pushPresentationChannel.invokeMethod<void>(
      'syncPushSnapshot',
      arguments,
    );
    pushPresentationCacheError.value = null;
  } on MissingPluginException {
    // Non-Runner embeddings do not provide the native snapshot bridge.
  } catch (error, stackTrace) {
    pushPresentationCacheError.value = error.toString();
    debugPrint('Push presentation cache update failed: $error');
    debugPrintStack(stackTrace: stackTrace);
  }
}

@visibleForTesting
bool isCacheablePushAvatarSource(String value) {
  final trimmed = value.trim();
  if (trimmed.startsWith('data:image/')) {
    try {
      final data = UriData.parse(trimmed);
      return data.mimeType.startsWith('image/') &&
          data.mimeType != 'image/svg+xml' &&
          data.contentAsBytes().isNotEmpty;
    } on FormatException {
      return false;
    }
  }
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

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
Future<void> _presentationExportTail = Future.value();
const _maximumPresentationExports = 8;
// Match BuzzPushPresentationCacheStore's per-write admission limits.
const _maximumProfilesPerWrite = 256;
const _maximumChannelsPerWrite = 512;
int _outstandingPresentationExports = 0;

/// Temporary admission failure; detached producers must retain and retry input.
class PushPresentationExportQueueFull extends StateError {
  /// Creates a saturation error for the shared eight-export budget.
  PushPresentationExportQueueFull()
    : super('Push presentation export queue is full (8 outstanding exports)');
}

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
/// Fails with [StateError] when eight exports are already outstanding.
/// Native persistence failures retry with bounded backoff, then propagate and
/// stop any remaining chunks. Other native errors propagate immediately.
Future<void> cacheBuzzPushProfileEvents(
  String communityID,
  Iterable<NostrEvent> events,
) async {
  if (defaultTargetPlatform != TargetPlatform.iOS || communityID.isEmpty) {
    return;
  }
  final batch = events.toList(growable: false);
  if (batch.isEmpty) return;
  await _serializePresentationExport(() async {
    final verified = await compute(
      _selectPushProfileEvents,
      batch,
      debugLabel: 'buzz-push-profile-cache',
    );
    if (verified.isEmpty) return;
    final retryBudget = _NativeWriteRetryBudget();
    for (final chunk in _boundedChunks(verified, _maximumProfilesPerWrite)) {
      await _invokeVerifiedChunk({
        'section': 'profiles',
        'communityId': communityID,
        'events': [for (final event in chunk) event.toJson()],
      }, retryBudget);
    }
  });
}

/// Exports verified channel metadata and membership for native authority checks.
/// Fails with [StateError] when eight exports are already outstanding.
/// Native persistence failures retry with bounded backoff, then propagate and
/// stop any remaining chunks. Other native errors propagate immediately.
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
  final batch = (
    metadata: metadataEvents.toList(growable: false),
    membership: membershipEvents.toList(growable: false),
  );
  if (batch.metadata.isEmpty && batch.membership.isEmpty) return;
  await _serializePresentationExport(() async {
    final verified = await compute(
      _selectPushChannelBatch,
      batch,
      debugLabel: 'buzz-push-channel-cache',
    );
    if (verified.metadata.isEmpty && verified.membership.isEmpty) return;
    final metadata = {
      for (final event in verified.metadata) event.getTagValue('d')!: event,
    };
    final membership = {
      for (final event in verified.membership) event.getTagValue('d')!: event,
    };
    final channelIDs = {...metadata.keys, ...membership.keys}.toList();
    final retryBudget = _NativeWriteRetryBudget();
    // Keep each channel's metadata and membership in the same native write.
    // All chunks retain this export's FIFO slot until handoff is complete.
    for (final ids in _boundedChunks(channelIDs, _maximumChannelsPerWrite)) {
      await _invokeVerifiedChunk({
        'section': 'channels',
        'communityId': communityID,
        'metadataEvents': [
          for (final id in ids)
            if (metadata[id] case final event?) event.toJson(),
        ],
        'membershipEvents': [
          for (final id in ids)
            if (membership[id] case final event?) event.toJson(),
        ],
      }, retryBudget);
    }
  });
}

Iterable<List<T>> _boundedChunks<T>(List<T> values, int maximum) sync* {
  for (var start = 0; start < values.length; start += maximum) {
    final end = start + maximum;
    yield values.sublist(start, end < values.length ? end : values.length);
  }
}

// Share one worker slot across profile/channel exports and retain FIFO native
// handoff, even across community changes. The two producers are coalesced profile
// fetches and channel refreshes. Eight outstanding batches allow a short burst
// while bounding retained batch count; individual batch sizes remain caller-owned.
// Saturation fails explicitly rather than acknowledging an export we cannot retain.
Future<void> _serializePresentationExport(
  Future<void> Function() export,
) async {
  if (_outstandingPresentationExports >= _maximumPresentationExports) {
    throw PushPresentationExportQueueFull();
  }
  _outstandingPresentationExports++;
  final previous = _presentationExportTail;
  final release = Completer<void>();
  _presentationExportTail = release.future;
  await previous;
  try {
    await export();
  } finally {
    // Keep the queue usable while propagating a worker failure to its caller.
    _outstandingPresentationExports--;
    release.complete();
  }
}

List<NostrEvent> _selectPushProfileEvents(List<NostrEvent> events) =>
    _newestVerifiedEvents(
      events,
      kind: 0,
      scope: (event) => event.pubkey.toLowerCase(),
    ).values.toList();

({List<NostrEvent> metadata, List<NostrEvent> membership})
_selectPushChannelBatch(
  ({List<NostrEvent> metadata, List<NostrEvent> membership}) batch,
) => selectPushChannelEvents(batch.metadata, batch.membership);

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
    await _invokeSnapshot({
      'section': 'avatar',
      'communityId': communityID,
      'sourceUrl': sourceURL,
      'png': png,
    }, bestEffort: true);
  } finally {
    release.complete();
  }
}

// One bounded backoff budget for the entire export, not one per chunk.
class _NativeWriteRetryBudget {
  static const _delays = [250, 500, 1000, 2000, 4000];
  int _used = 0;

  Duration? takeDelay() =>
      _used == _delays.length ? null : Duration(milliseconds: _delays[_used++]);
}

// Retry only native persistence failures, retaining the verified payload and
// the surrounding export's FIFO slot. Never repeat relay reads or verification.
Future<void> _invokeVerifiedChunk(
  Map<String, Object> arguments,
  _NativeWriteRetryBudget budget,
) async {
  final retryableCode = arguments['section'] == 'profiles'
      ? 'profile_cache_failed'
      : 'channel_cache_failed';
  while (true) {
    try {
      await _invokeSnapshot(arguments);
      return;
    } on PlatformException catch (error) {
      if (error.code != retryableCode) rethrow;
      final delay = budget.takeDelay();
      if (delay == null) rethrow;
      await Future<void>.delayed(delay);
    }
  }
}

Future<void> _invokeSnapshot(
  Map<String, Object> arguments, {
  bool bestEffort = false,
}) async {
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
    // Event exports must stop at the failed chunk and report failure to their
    // owner. Avatar updates preserve their existing detached best-effort policy.
    if (!bestEffort) rethrow;
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

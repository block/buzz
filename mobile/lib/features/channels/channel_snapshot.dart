part of 'channels_provider.dart';

extension _ChannelSnapshots on ChannelsNotifier {
  ChannelListSnapshotStorage? _snapshotStorage() =>
      _lifecycleRef.read(channelListSnapshotStorageProvider);

  List<Channel>? _restoreSnapshot(String? pubkey, String relay) {
    if (pubkey == null) return null;
    final storage = _snapshotStorage();
    final snapshot = storage?.read(pubkey, relay);
    if (snapshot == null) return null;
    try {
      final channels = snapshot.channels
          .map(Channel.fromJson)
          .where(
            (channel) =>
                channel.isMember && !snapshot.hiddenDmIds.contains(channel.id),
          )
          .toList();
      _hiddenDmIds = Set.unmodifiable(snapshot.hiddenDmIds);
      _hasLoaded = true;
      _hasCachedSnapshot = true;
      _needsSnapshotRevalidation = true;
      return channels;
    } catch (_) {
      unawaited(
        storage!.remove(pubkey, relay).catchError((Object error) {
          debugPrint(
            '[ChannelsNotifier] invalid snapshot removal failed: $error',
          );
        }),
      );
      return null;
    }
  }

  void Function(List<Channel>, Set<String>)? _captureSnapshotWriter(
    String pubkey,
    String relay,
  ) {
    final storage = _snapshotStorage();
    if (storage == null) return null;
    // Capture removal ownership before network I/O. Community removal retires
    // this token even while the active provider is still being torn down.
    final token = storage.capture(pubkey, relay);
    return (channels, hiddenDmIds) {
      final snapshot = ChannelListSnapshot(
        channels: [
          for (final channel in channels)
            if (channel.isMember) channel.toJson(),
        ],
        hiddenDmIds: hiddenDmIds,
      );
      unawaited(
        storage
            .write(token, snapshot)
            .then(
              (saved) {
                if (!saved) {
                  debugPrint('[ChannelsNotifier] snapshot was not saved');
                }
              },
              onError: (Object error, StackTrace stack) {
                debugPrint('[ChannelsNotifier] snapshot write failed: $error');
              },
            ),
      );
    };
  }
}

part of 'channels_provider.dart';

extension _HuddleCreationReceipts on ChannelsNotifier {
  Future<void> _reconcileHuddleReceipts(
    RelaySessionNotifier session,
    List<ChannelData> metadata,
    int generation,
  ) async {
    bool current() => _lifecycleRef.mounted && generation == _buildGeneration;
    if (!current()) return;
    if (_huddleReceiptsReading) {
      // Keep only the newest same-scope list while the current pass drains.
      _pendingHuddleReceiptMetadata = metadata;
      return;
    }
    _huddleReceiptsReading = true;
    final ids = [
      for (final channel in metadata)
        if (channel.channelType == 'stream' &&
            channel.visibility == 'private' &&
            !_checkedHuddleReceiptIds.contains(channel.id))
          channel.id,
    ];
    try {
      for (
        var start = 0;
        start < ids.length;
        start += kMaxExplicitChannelValues
      ) {
        // A notifier can be rebuilt for another relay while a batch is waiting.
        // Never send the next batch through a retired session scope.
        if (!current()) return;
        final batch = ids.sublist(
          start,
          min(start + kMaxExplicitChannelValues, ids.length),
        );
        try {
          final events = await session.fetchHistory(
            NostrFilter(
              kinds: const [9007],
              tags: {'#h': batch},
              limit: batch.length,
            ),
          );
          if (!current()) return;
          _checkedHuddleReceiptIds.addAll(batch);
          final verified = createdHuddleChannelIds(events, metadata);
          _verifiedHuddleIds.addAll(verified);
          final listed = _listState.value;
          if (listed == null || verified.isEmpty) continue;
          final visible = listed
              .where(
                (channel) =>
                    !(channel.channelType == 'stream' &&
                        channel.visibility == 'private' &&
                        _verifiedHuddleIds.contains(channel.id)),
              )
              .toList();
          if (visible.length != listed.length) {
            _listState = AsyncData(visible);
            _saveSnapshot?.call(visible, _hiddenDmIds);
          }
        } catch (_) {
          // Keep verified classifications; retry missing receipts on refresh.
          if (!current()) return;
        }
      }
    } finally {
      if (current()) {
        _huddleReceiptsReading = false;
        final pending = _pendingHuddleReceiptMetadata;
        _pendingHuddleReceiptMetadata = null;
        if (pending != null) {
          unawaited(_reconcileHuddleReceipts(session, pending, generation));
        }
      }
    }
  }
}

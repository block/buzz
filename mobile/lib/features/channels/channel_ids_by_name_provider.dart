import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'channels_provider.dart';

/// Lowercased channel name to channel id, for `#channel` links.
///
/// `channelsProvider` publishes a new list on every live event, so watching it
/// rebuilds every message row. This notifies only when a name or id changes.
final channelIdsByNameProvider =
    NotifierProvider<ChannelIdsByName, Map<String, String>>(
      ChannelIdsByName.new,
    );

class ChannelIdsByName extends Notifier<Map<String, String>> {
  @override
  Map<String, String> build() => Map.unmodifiable({
    for (final channel in ref.watch(channelsProvider).value ?? const [])
      channel.name.toLowerCase(): channel.id,
  });

  @override
  bool updateShouldNotify(
    Map<String, String> previous,
    Map<String, String> next,
  ) => !mapEquals(previous, next);
}

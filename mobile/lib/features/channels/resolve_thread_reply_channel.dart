import 'channel.dart';

/// Resolves the channel to send a thread reply against.
///
/// [channel] is normally already available (from `ref.watch(channelsProvider)`
/// at build time), but it can still be `null` right when the user sends — a
/// cold start or a notification/deep-link tap straight into a thread, before
/// the channel list has finished loading. Falling back to `null` in that case
/// would silently skip DM auto-mention (`SendMessage.call` only adds
/// DM-recipient `p` tags when a non-null, `isDm` channel is passed), so this
/// waits for [loadChannels] to resolve instead of sending with a channel
/// that's known to be missing.
Future<Channel?> resolveThreadReplyChannel({
  required Channel? channel,
  required String channelId,
  required Future<List<Channel>> Function() loadChannels,
}) async {
  if (channel != null) return channel;
  final channels = await loadChannels();
  return channels.where((c) => c.id == channelId).firstOrNull;
}

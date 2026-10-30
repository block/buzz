import '../channel.dart';

final _channelRef = RegExp(r'(?:^|\s)#([\p{L}\p{N}_-]+)', unicode: true);

/// The `#name` references in [args] split into the visible channels they name,
/// in order, and the names that match none. Direct messages never match.
({List<Channel> channels, List<String> unresolved}) resolveSlashChannelRefs(
  String args,
  Iterable<Channel> visible,
) {
  final byName = <String, Channel>{};
  for (final channel in visible) {
    if (channel.isDm) continue;
    byName.putIfAbsent(channel.name.toLowerCase(), () => channel);
  }
  final channels = <Channel>[];
  final unresolved = <String>[];
  for (final match in _channelRef.allMatches(args)) {
    final name = match.group(1)!;
    final channel = byName[name.toLowerCase()];
    if (channel == null) {
      unresolved.add(name);
    } else if (!channels.contains(channel)) {
      channels.add(channel);
    }
  }
  return (channels: channels, unresolved: unresolved);
}

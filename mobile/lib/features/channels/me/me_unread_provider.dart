import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../../../shared/me/me_sync.dart';
import '../../../shared/read_state/read_state_provider.dart';
import '../channel_unread.dart';
import '../channels_provider.dart';

/// Solo uses the same observed events and read marks as Home, before tab visits.
final meUnreadChannelIdsProvider = Provider<Set<String>>((ref) {
  final organization = ref.watch(meGroupsProvider).value;
  final channels = ref.watch(channelsProvider).value ?? [];
  final reads = ref.watch(readStateProvider);
  if (organization == null) return const {};
  return computeUnreadChannelIds(
    channels: channels.where(
      (c) =>
          c.isMember && !c.isArchived && organization.channels.contains(c.id),
    ),
    readState: reads,
    channelsNotifier: ref.read(channelsProvider.notifier),
  );
});

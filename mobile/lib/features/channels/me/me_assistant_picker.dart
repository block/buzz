import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/me/me_sync.dart';
import '../../../shared/me/me_assistants.dart';
import '../channel_management_provider.dart';

/// Revalidates current membership and ownership before personal chat delivery.
Future<List<String>> meRecipients(
  WidgetRef ref,
  String channelId,
  String? selected,
  List<String> explicit,
) async {
  final sync = ref.read(meSyncProvider);
  final recipients = explicit.isNotEmpty ? explicit : [?selected];
  if (recipients.isEmpty) {
    throw StateError('Choose an agent in the chat header before sending.');
  }
  sync.check();
  for (final recipient in recipients.toSet()) {
    ref.invalidate(meAssistantOwnershipProvider(recipient));
  }
  ref.invalidate(channelMembersProvider(channelId));
  // Start both independent checks together instead of scanning the directory
  // and then fetching membership on every message.
  final (ownership, members) = await (
    Future.wait([
      for (final recipient in recipients)
        ref.read(meAssistantOwnershipProvider(recipient).future),
    ]),
    ref.read(channelMembersProvider(channelId).future),
  ).wait;
  sync.check();
  if (ownership.any((owned) => !owned) ||
      recipients.any((key) => !members.any((member) => member.pubkey == key))) {
    throw StateError('Choose an available agent in this chat before sending.');
  }
  return recipients;
}

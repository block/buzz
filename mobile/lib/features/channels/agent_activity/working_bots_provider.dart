import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../channel_management_provider.dart';
import '../channel_typing_provider.dart';

/// Derived provider that computes which bot members in a channel are currently
/// typing (i.e. "working"). Returns a set of lowercase pubkeys.
///
/// Used by both the members button badge and the members sheet to avoid
/// duplicating the bot-typing cross-reference logic.
final workingBotPubkeysProvider = Provider.autoDispose
    .family<Set<String>, String>((ref, channelId) {
      final typingEntries = ref.watch(channelTypingProvider(channelId));
      final membersAsync = ref.watch(channelMembersProvider(channelId));
      final allMembers = membersAsync.asData?.value ?? const <ChannelMember>[];

      final botPubkeys = <String>{
        for (final m in allMembers)
          if (m.isBot) m.pubkey.toLowerCase(),
      };

      return <String>{
        for (final e in typingEntries)
          if (botPubkeys.contains(e.pubkey.toLowerCase()))
            e.pubkey.toLowerCase(),
      };
    });

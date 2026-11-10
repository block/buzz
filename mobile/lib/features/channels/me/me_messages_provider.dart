import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../../../shared/relay/relay.dart';
import '../channel_messages_provider.dart';

/// Me reads roots and replies as a continuous conversation.
final meMessagesProvider =
    NotifierProvider.family<
      ChannelMessagesNotifier,
      AsyncValue<List<NostrEvent>>,
      String
    >((id) => ChannelMessagesNotifier(id, flatTimeline: true));

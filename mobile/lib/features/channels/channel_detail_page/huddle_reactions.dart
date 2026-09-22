part of '../channel_detail_page.dart';

final _huddleAvatarOriginRegistryProvider = Provider(
  (_) => _HuddleAvatarOriginRegistry(),
);

class _HuddleAvatarOriginRegistry {
  final Map<String, GlobalKey> _keys = {};

  void register(String pubkey, GlobalKey key) {
    _keys[pubkey.toLowerCase()] = key;
  }

  void unregister(String pubkey, GlobalKey key) {
    final normalized = pubkey.toLowerCase();
    if (identical(_keys[normalized], key)) _keys.remove(normalized);
  }

  Offset? originFor(String? pubkey) {
    if (pubkey == null) return null;
    final context = _keys[pubkey.toLowerCase()]?.currentContext;
    final box = context?.findRenderObject();
    if (box is! RenderBox || !box.hasSize) return null;
    return box.localToGlobal(box.size.center(Offset.zero));
  }
}

void _burstHuddleReaction({
  required WidgetRef ref,
  required BuildContext fallbackContext,
  required String emoji,
  required String? senderPubkey,
}) {
  if (MediaQuery.maybeDisableAnimationsOf(fallbackContext) ?? false) return;
  final origin = ref
      .read(_huddleAvatarOriginRegistryProvider)
      .originFor(senderPubkey);
  if (origin == null) {
    burstEmojiFromContext(ref, fallbackContext, emoji, requirePositive: false);
    return;
  }
  ref.read(emojiBurstControllerProvider).burst(emoji, origin);
}

void _useIncomingHuddleReactions({
  required BuildContext context,
  required WidgetRef ref,
  required bool inSession,
  required String? channelId,
  required String? currentPubkey,
  required SessionStatus relayStatus,
}) {
  useEffect(() {
    if (!inSession ||
        channelId == null ||
        relayStatus != SessionStatus.connected) {
      return null;
    }

    final relaySession = ref.read(relaySessionProvider.notifier);
    final seenEventIds = <String>{};
    var disposed = false;
    VoidCallback? unsubscribe;
    Future.microtask(() async {
      try {
        final cleanup = await relaySession.subscribe(
          NostrFilter(
            kinds: const [EventKind.huddleReaction],
            tags: {
              '#h': [channelId],
            },
            limit: 1000,
          ).copyWithSince(DateTime.now().millisecondsSinceEpoch ~/ 1000),
          (event) {
            if (disposed || !seenEventIds.add(event.id)) return;
            if (event.pubkey.toLowerCase() == currentPubkey?.toLowerCase()) {
              return;
            }
            final emoji = _huddleReactionEmoji(event);
            if (emoji == null || !context.mounted) return;
            _burstHuddleReaction(
              ref: ref,
              fallbackContext: context,
              emoji: emoji,
              senderPubkey: event.pubkey,
            );
          },
        );
        if (disposed) {
          cleanup();
        } else {
          unsubscribe = cleanup;
        }
      } catch (error) {
        if (!disposed) {
          debugPrint('[MobileHuddle] reaction subscription failed: $error');
        }
      }
    });

    return () {
      disposed = true;
      unsubscribe?.call();
    };
  }, [inSession, channelId, currentPubkey, relayStatus]);
}

String? _huddleReactionEmoji(NostrEvent event) {
  if (event.kind != EventKind.huddleReaction) return null;
  for (final tag in event.tags) {
    if (tag.length < 2 || tag.first != 'reaction') continue;
    final emoji = tag[1].trim();
    if (emoji.isNotEmpty) return emoji;
  }
  final emoji = event.content.trim();
  return emoji.isEmpty ? null : emoji;
}

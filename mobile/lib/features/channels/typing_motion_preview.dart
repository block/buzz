import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import 'channel_typing_provider.dart';
import 'inline_agent_typing.dart';

/// Install-only motion playground. Never publishes typing events.
class TypingMotionPreview extends HookConsumerWidget {
  const TypingMotionPreview({super.key, required this.channelId});
  final String channelId;
  static const enabled = bool.fromEnvironment('BUZZ_TYPING_PREVIEW');

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final count = useState(1);
    final playing = useState(true);
    final reduced = MediaQuery.disableAnimationsOf(context);
    final profiles = ref.watch(userCacheProvider);
    final bots = ref.watch(agentMentionPubkeysProvider(channelId));
    final agentKeys = {
      for (final profile in profiles.values)
        if (profile.isAgent || bots.contains(profile.pubkey)) profile.pubkey,
      ...bots,
    }.take(8).toList();
    const demoKeys = ['typing-preview-agent-1', 'typing-preview-agent-2'];
    agentKeys.insertAll(0, demoKeys);
    final people = profiles.keys
        .where(
          (key) => !bots.contains(key) && !(profiles[key]?.isAgent ?? false),
        )
        .take(8)
        .toList();
    // Deliberately alternate shapes, rather than letting cache insertion order
    // fill every demo slot with people before an agent can appear.
    final keys = <String>[];
    for (
      var i = 0;
      keys.length < 8 && (i < people.length || i < agentKeys.length);
      i++
    ) {
      if (i < people.length) keys.add(people[i]);
      if (keys.length < 8 && i < agentKeys.length) keys.add(agentKeys[i]);
    }
    useEffect(() {
      if (!playing.value || reduced || keys.isEmpty) return null;
      final timer = Timer.periodic(const Duration(seconds: 2), (_) {
        count.value = count.value >= keys.length ? 1 : count.value + 1;
      });
      return timer.cancel;
    }, [playing.value, reduced, keys.length]);
    final shown = count.value.clamp(1, keys.isEmpty ? 1 : keys.length);
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          children: [
            Flexible(
              child: Text(
                'Typing preview · $shown · demo agents',
                style: context.textTheme.labelSmall,
              ),
            ),
            IconButton(
              tooltip: playing.value ? 'Pause preview' : 'Play preview',
              onPressed: () => playing.value = !playing.value,
              icon: Icon(playing.value ? Icons.pause : Icons.play_arrow),
            ),
            IconButton(
              tooltip: 'Fewer people',
              onPressed: () {
                playing.value = false;
                count.value = shown > 1 ? shown - 1 : 1;
              },
              icon: const Icon(Icons.remove),
            ),
            IconButton(
              tooltip: 'More people',
              onPressed: () {
                playing.value = false;
                count.value = shown < keys.length ? shown + 1 : shown;
              },
              icon: const Icon(Icons.add),
            ),
          ],
        ),
        if (keys.isNotEmpty)
          InlineAgentTyping(
            channelId: channelId,
            previewAvatars: {
              for (final key in demoKeys)
                key: AvatarImage(
                  imageUrl: null,
                  radius: messageAvatarSize / 2,
                  isAgent: true,
                  backgroundColor: context.colors.primaryContainer,
                  fallback: Icon(
                    Icons.smart_toy_rounded,
                    color: context.colors.onPrimaryContainer,
                    size: 24,
                  ),
                ),
            },
            entries: [
              for (final key in keys.take(shown))
                TypingEntry(pubkey: key, expiresAtMs: 0),
            ],
          )
        else
          const Text('Waiting for community profiles…'),
      ],
    );
  }
}

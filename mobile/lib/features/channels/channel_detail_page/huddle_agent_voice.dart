part of '../channel_detail_page.dart';

class _HuddleAgentVoice extends HookConsumerWidget {
  const _HuddleAgentVoice({
    required this.parentChannelId,
    required this.ephemeralChannelId,
  });

  final String parentChannelId;
  final String ephemeralChannelId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (defaultTargetPlatform != TargetPlatform.iOS) {
      return const SizedBox.shrink();
    }

    final speech = useMemoized(HuddleSpeech.new);
    final selected = useState<String?>(null);
    final status = useState<String>('Add an agent to speak');
    final selecting = useRef(false);
    final selectedAt = useRef(0);
    final heard = useMemoized(() => <String>{});
    final parentMembers =
        ref.watch(channelMembersProvider(parentChannelId)).asData?.value ??
        const <ChannelMember>[];
    final agents = [
      for (final entry
          in ref.watch(agentDirectoryProvider).asData?.value ??
              const <AgentDirectoryEntry>[])
        if (parentMembers.any(
              (member) =>
                  member.isBot &&
                  member.pubkey.toLowerCase() == entry.pubkey.toLowerCase(),
            ) &&
            (entry.channelIds.isEmpty ||
                entry.channelIds.contains(parentChannelId)))
          entry,
    ];
    final eligiblePubkeys = {
      for (final entry in agents) entry.pubkey.toLowerCase(),
    };
    final childMembers =
        ref.watch(channelMembersProvider(ephemeralChannelId)).asData?.value ??
        const <ChannelMember>[];
    final bots = [
      for (final member in childMembers)
        if (member.isBot &&
            eligiblePubkeys.contains(member.pubkey.toLowerCase()))
          member.pubkey.toLowerCase(),
    ];

    Future<void> selectAgent(String pubkey) async {
      if (selecting.value ||
          !eligiblePubkeys.contains(pubkey) ||
          (bots.isNotEmpty && !bots.contains(pubkey))) {
        return;
      }
      selecting.value = true;
      status.value = 'Joining agent';
      try {
        final actions = ref.read(channelActionsProvider);
        if (!bots.contains(pubkey)) {
          await actions.addMembers(
            channelId: ephemeralChannelId,
            pubkeys: [pubkey],
            role: 'bot',
          );
        }
        if (!context.mounted) return;
        selectedAt.value = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        selected.value = pubkey;
        await speech.start();
        if (context.mounted) status.value = 'Listening on this device';
      } catch (error) {
        if (context.mounted) {
          selected.value = null;
          status.value = error is PlatformException
              ? error.message ?? error.code
              : error.toString();
        }
      } finally {
        selecting.value = false;
      }
    }

    useEffect(() {
      if (bots.length == 1 && selected.value == null) {
        unawaited(
          Future.microtask(() async {
            if (context.mounted) await selectAgent(bots.first);
          }),
        );
      }
      return null;
    }, [bots.join(',')]);

    useEffect(() {
      return () {
        unawaited(
          speech.stop().catchError((Object error) {
            debugPrint('[HuddleSpeech] stop failed: $error');
          }),
        );
        speech.dispose();
      };
    }, [speech]);

    speech.onTranscript = (text) {
      if (!context.mounted) return;
      final agent = selected.value;
      if (agent == null || ref.read(huddleSessionProvider).isMuted) return;
      status.value = 'Sending speech';
      unawaited(
        ref
            .read(sendMessageProvider)
            .call(
              channelId: ephemeralChannelId,
              content: text,
              mentionPubkeys: [agent],
            )
            .then((_) {
              if (context.mounted) status.value = 'Waiting for agent';
            })
            .catchError((Object error) {
              if (context.mounted) {
                status.value = 'Could not send speech: $error';
              }
            }),
      );
    };

    speech.onError = (message) {
      if (context.mounted) status.value = message;
    };

    ref.listen(channelMessagesProvider(ephemeralChannelId), (previous, next) {
      final agent = selected.value;
      if (agent == null) return;
      for (final event in next.asData?.value ?? const <NostrEvent>[]) {
        if (event.pubkey.toLowerCase() != agent ||
            event.kind != EventKind.streamMessage ||
            event.createdAt < selectedAt.value ||
            !heard.add(event.id)) {
          continue;
        }
        status.value = 'Agent speaking';
        unawaited(
          speech.speak(event.content).catchError((Object error) {
            if (context.mounted) status.value = 'Could not speak reply: $error';
          }),
        );
      }
    });

    final agent = selected.value;
    final selectedName = agents
        .where((entry) => entry.pubkey.toLowerCase() == agent)
        .map((entry) => entry.displayName ?? entry.pubkey.substring(0, 8))
        .firstOrNull;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.sm),
      child: Column(
        children: [
          if (agent == null && bots.isEmpty && agents.isEmpty)
            const Text('No agents in this channel')
          else if (agent == null && bots.isEmpty)
            PopupMenuButton<String>(
              tooltip: 'Add an agent to this Huddle',
              onSelected: (pubkey) => unawaited(selectAgent(pubkey)),
              itemBuilder: (_) => [
                for (final entry in agents)
                  PopupMenuItem(
                    value: entry.pubkey.toLowerCase(),
                    child: Text(
                      entry.displayName ?? entry.pubkey.substring(0, 8),
                    ),
                  ),
              ],
              child: const Text('Add agent'),
            )
          else if (agent == null)
            TextButton(
              onPressed: () => unawaited(selectAgent(bots.first)),
              child: const Text('Start agent speech'),
            )
          else
            Text(selectedName ?? 'Agent', style: context.textTheme.titleSmall),
          Semantics(
            liveRegion: true,
            child: Text(status.value, style: context.textTheme.bodySmall),
          ),
        ],
      ),
    );
  }
}

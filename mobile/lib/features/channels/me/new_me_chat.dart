part of 'me_page.dart';

/// Opens the New chat draft, first offering to resume or discard a creation
/// that was interrupted before it finished.
Future<void> _openNewMeChat(BuildContext context, WidgetRef ref) async {
  final initialSync = ref.read(meSyncProvider);
  final unfinished = MeChatCreation.pending(initialSync);
  if (unfinished != null) {
    final resume = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Unfinished chat'),
        content: Text(
          'Resume “${unfinished.name}”, or discard its setup to start another chat. Any channel and messages already created are kept in Home or Solo.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Later'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Discard setup'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Resume'),
          ),
        ],
      ),
    );
    if (resume == null || !context.mounted) return;
    if (!resume) {
      await unfinished.discard(initialSync);
      ref.invalidate(channelsProvider);
    }
  }
  if (!context.mounted) return;
  await Navigator.of(
    context,
  ).push<void>(MaterialPageRoute(builder: (_) => const _MeChatDraft()));
}

Future<Channel> _createMeChat(
  WidgetRef ref,
  MeSync sync,
  String channelId,
  String name,
  String assistant,
  String? project, {
  String? initialContent,
  List<List<String>> initialMediaTags = const [],
}) async {
  sync = _currentMeSync(ref, sync);
  final pending = MeChatCreation.pending(sync);
  final initialMessage = initialContent == null
      ? null
      : pending?.initialMessage ??
            NostrEvent.fromJson(
              signNostrEvent(
                kind: EventKind.streamMessage,
                content: initialContent,
                tags: [
                  ['h', channelId],
                  ['p', assistant],
                  ...initialMediaTags,
                ],
                secretKey: sync.privateHex,
              ).toMap(),
            );
  final intent = MeChatCreation(
    channelId,
    name,
    assistant,
    project: project,
    initialMessage: initialMessage,
  );
  await intent.save(sync);
  final actions = ref.read(channelActionsProvider);
  final preflight = await Future.wait<Object?>([
    isOwnedMeAssistant(sync, assistant),
    sync.head(),
    actions.findJoinedChannel(channelId),
  ]);
  final owned = preflight[0] as bool;
  sync.check();
  if (!owned) {
    throw StateError('This agent is no longer available.');
  }
  final head = preflight[1] as NostrEvent?;
  (head == null ? const MeGroups() : sync.decode(head))
      .place(channelId, group: project)
      .encode(sync.community);
  return intent.complete<Channel>(
    sync,
    findChannel: () async => preflight[2] as Channel?,
    createChannel: () => actions.createChannel(
      channelId: channelId,
      name: name,
      channelType: 'stream',
      visibility: 'private',
      labels: const ['buzz.solo'],
      waitForChannelList: false,
    ),
    hasAgent: () async {
      ref.invalidate(channelMembersProvider(channelId));
      final members = await ref.read(channelMembersProvider(channelId).future);
      return members.any((m) => m.pubkey == assistant);
    },
    addAgent: () =>
        actions.addMembers(channelId: channelId, pubkeys: [assistant]),
    isPlaced: () async {
      final latest = await sync.head();
      if (latest == null) return false;
      final groups = sync.decode(latest);
      return groups.channels.contains(channelId) &&
          (project == null || groups.assignments[channelId] == project);
    },
    place: () =>
        sync.update((current) => current.place(channelId, group: project)),
  );
}

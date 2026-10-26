part of 'me_page.dart';

Future<(Channel, String)?> _showNewMeChat(
  BuildContext context,
  WidgetRef ref,
) async {
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
    if (resume == null || !context.mounted) return null;
    if (!resume) {
      await unfinished.discard(initialSync);
      ref.invalidate(channelsProvider);
    }
  }
  if (!context.mounted) return null;
  if (defaultTargetPlatform == TargetPlatform.iOS) {
    final sync = ref.read(meSyncProvider);
    final pending = MeChatCreation.pending(sync);
    final id = pending?.id ?? const Uuid().v4();
    String name = pending?.name ?? '';
    String? assistant = pending?.agent;
    String? project = pending?.project;
    String? error = pending == null ? null : 'Resume your unfinished chat.';
    var retry = pending != null;
    while (context.mounted) {
      sync.check();
      final directory = await ref.read(meAssistantsProvider.future);
      if (!context.mounted) return null;
      sync.check();
      if (!context.mounted) return null;
      final theme = utilitySurfaceThemeData(Theme.of(context));
      Map<Object?, Object?>? draft;
      try {
        draft = await const MethodChannel('buzz/profile_text_editor')
            .invokeMethod<Map<Object?, Object?>>('presentChat', {
              'title': 'New chat',
              'initialValue': name,
              'placeholder': 'Chat name',
              'multiline': false,
              'brightness': theme.brightness.name,
              'pageBackgroundArgb': theme.colorScheme.surface.toARGB32(),
              'containerBackgroundArgb': theme
                  .colorScheme
                  .surfaceContainerHighest
                  .toARGB32(),
              'containerCornerRadius': Radii.container,
              'selectedAssistant': assistant,
              'selectedProject': project,
              'projects': [
                for (final group
                    in ref.read(meGroupsProvider).requireValue.groups)
                  {'id': group['id'], 'name': group['name']},
              ],
              'fieldsLocked': retry,
              'error': error,
              'assistants': [
                for (final agent in directory)
                  {
                    'id': agent.pubkey,
                    'name': agent.displayName ?? 'Agent',
                    'artworkScale':
                        usesInsetAgentArtwork(
                          ref
                              .read(userCacheProvider.notifier)
                              .get(agent.pubkey)
                              ?.avatarUrl,
                        )
                        ? '$animatedAvatarArtworkScale'
                        : '1',
                    'avatarUrl':
                        ref
                            .read(userCacheProvider.notifier)
                            .get(agent.pubkey)
                            ?.avatarUrl ??
                        '',
                  },
              ],
            });
      } on MissingPluginException {
        break;
      }
      if (draft == null || !context.mounted) return null;
      name = (draft['name'] as String? ?? '').trim();
      assistant = draft['assistant'] as String?;
      project = draft['project'] as String?;
      if (project == '') project = null;
      if (name.isEmpty || assistant == null) return null;
      retry = true;
      try {
        final created = await _createMeChat(
          ref,
          sync,
          id,
          name,
          assistant,
          project,
        );
        return (created, assistant);
      } catch (failure) {
        if (failure is MeRecoveryConflict && context.mounted) {
          await _offerMeRecovery(context, sync);
        }
        error =
            '${failure.toString().replaceFirst('Bad state: ', '')}\nRetry continues this same chat.';
      }
    }
  }
  if (!context.mounted) return null;
  return showBuzzModalBottomSheet<(Channel, String)>(
    context: context,
    title: 'New chat',
    isScrollControlled: true,
    requestFocus: true,
    builder: (_) => const _NewMeChat(),
  );
}

Future<Channel> _createMeChat(
  WidgetRef ref,
  MeSync sync,
  String channelId,
  String name,
  String assistant,
  String? project,
) async {
  sync = _currentMeSync(ref, sync);
  final intent = MeChatCreation(channelId, name, assistant, project: project);
  await intent.save(sync);
  final owned = await isOwnedMeAssistant(sync, assistant);
  sync.check();
  if (!owned) {
    throw StateError('This agent is no longer available.');
  }
  final head = await sync.head();
  (head == null ? const MeGroups() : sync.decode(head))
      .place(channelId, group: project)
      .encode(sync.community);
  final actions = ref.read(channelActionsProvider);
  return intent.complete<Channel>(
    sync,
    findChannel: () async {
      final all = await ref.read(channelsProvider.notifier).fetchFresh();
      return all.where((c) => c.id == channelId).firstOrNull;
    },
    createChannel: () => actions.createChannel(
      channelId: channelId,
      name: name,
      channelType: 'stream',
      visibility: 'private',
      labels: const ['buzz.solo'],
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

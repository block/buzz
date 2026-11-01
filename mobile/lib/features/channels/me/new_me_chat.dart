part of 'me_page.dart';

class _NewMeChat extends HookConsumerWidget {
  const _NewMeChat();
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sync = ref.watch(meSyncProvider);
    final pending = useMemoized(() => MeChatCreation.pending(sync));
    final name = useTextEditingController(text: pending?.name ?? '');
    final selected = useState<String?>(pending?.agent);
    final project = useState<String?>(pending?.project);
    final groups = ref.watch(meGroupsProvider);
    final channelId = useMemoized(() => pending?.id ?? const Uuid().v4());
    final frozenName = useState<String?>(pending?.name);
    final frozenAgent = useState<String?>(pending?.agent);
    final busy = useState(false);
    final error = useState<String?>(
      pending == null ? null : 'Resume your unfinished chat.',
    );
    final directory = ref.watch(meAssistantsProvider);
    final originalScope = useMemoized(() => (sync.community, sync.pubkey));
    if (originalScope != (sync.community, sync.pubkey)) {
      return const Padding(
        padding: EdgeInsets.all(Grid.gutter),
        child: Text(
          'Your account or community changed. Return to Solo to start a chat.',
        ),
      );
    }
    final choices = directory.asData?.value ?? [];
    final profile = ref.watch(userCacheProvider)[selected.value];
    final chosen =
        choices.where((agent) => agent.pubkey == selected.value).firstOrNull ??
        (frozenAgent.value == null
            ? null
            : AgentDirectoryEntry(
                pubkey: frozenAgent.value!,
                displayName: profile?.label ?? frozenAgent.value!,
              ));
    return SingleChildScrollView(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        Grid.xxs,
        Grid.gutter,
        MediaQuery.viewInsetsOf(context).bottom + Grid.xs,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          TextField(
            key: const ValueKey('new-session-name'),
            controller: name,
            autofocus: true,
            textCapitalization: TextCapitalization.sentences,
            enabled: !busy.value && frozenName.value == null,
            maxLength: 120,
            decoration: const InputDecoration(hintText: 'Chat name'),
            onChanged: (_) => error.value = null,
          ),
          const SizedBox(height: Grid.xs),
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(
                child: Column(
                  children: [
                    Semantics(
                      label:
                          'Agent: ${chosen?.displayName ?? 'Choose an agent'}',
                      child: IconButton(
                        key: const ValueKey('new-session-assistant'),
                        tooltip:
                            'Agent: ${chosen?.displayName ?? 'Choose an agent'}',
                        icon: ExcludeSemantics(
                          child: chosen == null
                              ? const Icon(BuzzIcons.bot)
                              : AvatarImage(
                                  imageUrl: profile?.avatarUrl,
                                  radius: 18,
                                  isAgent: true,
                                  backgroundColor: context.controlSurface,
                                  fallback: Text(
                                    chosen.displayName?.trim().isNotEmpty ==
                                            true
                                        ? chosen.displayName!
                                              .trim()[0]
                                              .toUpperCase()
                                        : profile?.initial ?? 'A',
                                  ),
                                ),
                        ),
                        onPressed: busy.value || frozenName.value != null
                            ? null
                            : () async {
                                FocusScope.of(context).unfocus();
                                final picked =
                                    await showBuzzModalBottomSheet<String>(
                                      context: context,
                                      title: 'Agent',
                                      isScrollControlled: true,
                                      showDragHandle: true,
                                      builder: (_) => MeAssistantList(
                                        choices: choices,
                                        selected: selected.value,
                                      ),
                                    );
                                if (context.mounted && picked != null) {
                                  selected.value = picked;
                                }
                              },
                      ),
                    ),
                    Text(
                      chosen?.displayName ?? 'Choose an agent',
                      textAlign: TextAlign.center,
                    ),
                  ],
                ),
              ),
              Expanded(
                child: Column(
                  children: [
                    PopupMenuButton<String>(
                      key: const ValueKey('new-session-project'),
                      tooltip: 'Workspace',
                      icon: const Icon(BuzzIcons.folder),
                      initialValue: project.value ?? '',
                      enabled:
                          !busy.value &&
                          frozenName.value == null &&
                          groups.hasValue,
                      onSelected: (value) =>
                          project.value = value.isEmpty ? null : value,
                      itemBuilder: (_) => [
                        const PopupMenuItem(value: '', child: Text('Chats')),
                        for (final group
                            in groups.asData?.value.groups ??
                                <Map<String, dynamic>>[])
                          PopupMenuItem(
                            value: group['id'] as String,
                            child: Text(group['name'] as String),
                          ),
                      ],
                    ),
                    Text(
                      project.value == null
                          ? 'Chats'
                          : (groups.asData?.value.groups ??
                                        <Map<String, dynamic>>[])
                                    .where(
                                      (group) => group['id'] == project.value,
                                    )
                                    .map((group) => group['name'] as String)
                                    .firstOrNull ??
                                'Unavailable workspace',
                      textAlign: TextAlign.center,
                    ),
                  ],
                ),
              ),
            ],
          ),
          if (directory.isLoading) const LinearProgressIndicator(),
          if (directory.hasError)
            TextButton(
              onPressed: () {
                ref.invalidate(meAssistantsProvider);
              },
              child: const Text('Reload agents'),
            ),
          if (!directory.isLoading && !directory.hasError && choices.isEmpty)
            const Padding(
              padding: EdgeInsets.symmetric(vertical: Grid.gutter),
              child: Text(
                'No agents are available for this account. Connect an agent on desktop, then refresh.',
              ),
            ),
          if (error.value != null)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: Grid.gutter),
              child: Text(error.value!),
            ),
          const SizedBox(height: Grid.gutter),
          FilledButton(
            onPressed: busy.value
                ? null
                : () async {
                    if (name.text.trim().isEmpty || selected.value == null) {
                      error.value = 'Enter a name and choose an agent.';
                      return;
                    }
                    busy.value = true;
                    error.value = null;
                    try {
                      frozenName.value ??= name.text.trim();
                      frozenAgent.value ??= selected.value;
                      final created = await _createMeChat(
                        ref,
                        sync,
                        channelId,
                        frozenName.value!,
                        frozenAgent.value!,
                        project.value,
                      );
                      if (context.mounted) {
                        Navigator.of(
                          context,
                        ).pop((created, frozenAgent.value!));
                      }
                    } catch (e) {
                      if (e is MeRecoveryConflict && context.mounted) {
                        await _offerMeRecovery(context, sync);
                      }
                      if (context.mounted) {
                        error.value =
                            '${e.toString().replaceFirst('Bad state: ', '')}\nRetry continues this same chat.';
                      }
                    } finally {
                      if (context.mounted) busy.value = false;
                    }
                  },
            child: Text(
              busy.value
                  ? 'Creating chat…'
                  : frozenName.value != null
                  ? 'Retry'
                  : 'Create chat',
            ),
          ),
        ],
      ),
    );
  }
}

part of 'me_page.dart';

class _NewMeChat extends HookConsumerWidget {
  const _NewMeChat();
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sync = ref.watch(meSyncProvider);
    final pending = useMemoized(() => MeChatCreation.pending(sync));
    final messageMode = pending == null || pending.initialMessage != null;
    final name = useTextEditingController(
      text: pending?.initialMessage?.content ?? pending?.name ?? '',
    );
    final selected = useState<String?>(pending?.agent);
    final manuallySelected = useState(false);
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
    final bestie = ref.watch(defaultAgentProvider).value;
    useEffect(() {
      if (!manuallySelected.value && frozenAgent.value == null) {
        selected.value = choices.any((a) => a.pubkey == bestie) ? bestie : null;
      }
      return null;
    }, [bestie, choices]);
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
            maxLength: messageMode ? null : 120,
            minLines: messageMode ? 3 : 1,
            maxLines: messageMode ? 6 : 1,
            decoration: InputDecoration(
              labelText: messageMode ? 'Message' : 'Chat name',
              hintText: messageMode ? 'What would you like to work on?' : null,
            ),
            onChanged: (_) => error.value = null,
          ),
          const SizedBox(height: Grid.xs),
          DropdownButtonFormField<String>(
            key: const ValueKey('new-session-project'),
            initialValue: project.value ?? '',
            isExpanded: true,
            decoration: const InputDecoration(labelText: 'Project'),
            items: [
              const DropdownMenuItem(value: '', child: Text('No project')),
              for (final group
                  in groups.asData?.value.groups ?? <Map<String, dynamic>>[])
                DropdownMenuItem(
                  value: group['id'] as String,
                  child: Text(group['name'] as String),
                ),
              if (project.value != null &&
                  !(groups.asData?.value.groups ?? []).any(
                    (group) => group['id'] == project.value,
                  ))
                DropdownMenuItem(
                  value: project.value,
                  child: const Text('Unavailable project'),
                ),
            ],
            onChanged:
                busy.value || frozenName.value != null || !groups.hasValue
                ? null
                : (value) => project.value = value == '' ? null : value,
          ),
          const SizedBox(height: Grid.xs),
          AppListRowRaw(
            key: const ValueKey('new-session-assistant'),
            leading: chosen == null
                ? const Icon(BuzzIcons.plus)
                : AvatarImage(
                    imageUrl: profile?.avatarUrl,
                    radius: 20,
                    isAgent: true,
                    backgroundColor: context.controlSurface,
                    fallback: Text(
                      chosen.displayName?.trim().isNotEmpty == true
                          ? chosen.displayName!.trim()[0].toUpperCase()
                          : profile?.initial ?? 'A',
                    ),
                  ),
            title: Text(chosen?.displayName ?? 'Choose an agent'),
            trailing: const Icon(BuzzIcons.chevronRight, size: 20),
            onTap: busy.value || frozenName.value != null
                ? null
                : () async {
                    FocusScope.of(context).unfocus();
                    if (!context.mounted) return;
                    final picked = await showBuzzModalBottomSheet<String>(
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
                      manuallySelected.value = true;
                      selected.value = picked;
                    }
                  },
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
                      error.value = messageMode
                          ? 'Enter a message and choose an agent.'
                          : 'Enter a name and choose an agent.';
                      return;
                    }
                    if (messageMode &&
                        utf8.encode(name.text.trim()).length > 8192) {
                      error.value = 'Message is too long. Keep it under 8 KB.';
                      return;
                    }
                    busy.value = true;
                    error.value = null;
                    try {
                      frozenName.value ??= messageMode
                          ? startingMeChatTitle(name.text)
                          : name.text.trim();
                      frozenAgent.value ??= selected.value;
                      final created = await _createMeChat(
                        ref,
                        sync,
                        channelId,
                        frozenName.value!,
                        frozenAgent.value!,
                        project.value,
                        initialContent: messageMode ? name.text.trim() : null,
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
                  : messageMode
                  ? 'Send'
                  : 'Create chat',
            ),
          ),
        ],
      ),
    );
  }
}

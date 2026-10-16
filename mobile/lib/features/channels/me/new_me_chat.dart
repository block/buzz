part of 'me_page.dart';

class _NewMeChat extends HookConsumerWidget {
  const _NewMeChat();
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sync = ref.watch(meSyncProvider);
    final pending = useMemoized(() => MeChatCreation.pending(sync));
    final name = useTextEditingController(text: pending?.name ?? '');
    final selected = useState<String?>(pending?.agent);
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
    final chosen = choices
        .where((agent) => agent.pubkey == selected.value)
        .firstOrNull;
    final profile = ref.watch(userCacheProvider)[selected.value];
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
                      profile?.initial ??
                          (chosen.displayName?.isNotEmpty == true
                              ? chosen.displayName![0]
                              : 'A'),
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

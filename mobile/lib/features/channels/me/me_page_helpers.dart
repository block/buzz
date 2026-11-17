part of 'me_page.dart';

class _MeRetry extends StatelessWidget {
  final VoidCallback onRetry;
  const _MeRetry({required this.onRetry});
  @override
  Widget build(BuildContext context) => Center(
    child: Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        const Text('Couldn’t load your chats and projects.'),
        TextButton(onPressed: onRetry, child: const Text('Try again')),
      ],
    ),
  );
}

enum _MeArchivePhase { confirming, pending, accepted }

// Keep attempt identity across metadata observations so an older recovery
// cannot report feedback for a later archive. Revisions live only here.
class _MeArchiveOperation {
  _MeArchiveOperation(
    this.phase,
    this.activeRevision, {
    Object? attempt,
    this.archivedRevision,
  }) : attempt = attempt ?? Object();

  final _MeArchivePhase phase;
  final Object attempt;
  final DateTime activeRevision;
  final DateTime? archivedRevision;

  bool get needsRecovery =>
      phase == _MeArchivePhase.accepted && archivedRevision == null;

  _MeArchiveOperation withPhase(_MeArchivePhase next) => _MeArchiveOperation(
    next,
    activeRevision,
    attempt: attempt,
    archivedRevision: archivedRevision,
  );

  _MeArchiveOperation? observe(Channel channel) {
    final revision = channel.createdAt;
    final archived = archivedRevision;
    if (channel.isArchived) {
      // A prior lifecycle's archived metadata cannot reconcile this attempt.
      if (!revision.isAfter(activeRevision) ||
          (archived != null && !revision.isAfter(archived))) {
        return this;
      }
      return _MeArchiveOperation(
        phase,
        activeRevision,
        attempt: attempt,
        archivedRevision: revision,
      );
    }
    if (archived != null && revision.isAfter(archived)) {
      if (phase == _MeArchivePhase.accepted) return null;
      return _MeArchiveOperation(phase, revision, attempt: attempt);
    }
    if (!revision.isAfter(activeRevision)) return this;
    // Without an observed archive, even newer active metadata can precede
    // acceptance, even on the writer. Keep guarding until an archive is observed.
    return _MeArchiveOperation(
      phase,
      revision,
      attempt: attempt,
      archivedRevision: archived,
    );
  }
}

class _MeChatRow extends ConsumerWidget {
  const _MeChatRow({
    required this.channelId,
    required this.name,
    required this.onTap,
    required this.onLongPress,
    required this.isUnread,
    required this.canArchive,
    required this.archivePhase,
    required this.onRefresh,
    required this.onArchive,
  });
  final String channelId;
  final String name;
  final bool isUnread;
  final bool canArchive;
  final _MeArchivePhase? archivePhase;
  final Future<void> Function() onRefresh;
  final Future<void> Function() onArchive;
  final VoidCallback onTap;
  final VoidCallback onLongPress;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final archiving = archivePhase == _MeArchivePhase.pending;
    final accepted = archivePhase == _MeArchivePhase.accepted;
    Future<void> archive() async {
      if (!canArchive || archivePhase != null) return;
      await onArchive();
    }

    final color = channelNavigationForeground(context, unread: isUnread);
    final working = ref.watch(meWorkingAgentsProvider(channelId));
    final status = accepted
        ? ', archived, awaiting refresh'
        : archiving
        ? ', archiving'
        : working.isEmpty
        ? ''
        : ', ${working.join(', ')} working';
    final row = Semantics(
      label: '${isUnread ? '$name, unread' : name}$status',
      liveRegion: archiving || accepted,
      onTap: onTap,
      onLongPress: onLongPress,
      customSemanticsActions: {
        if (canArchive && archivePhase == null)
          const CustomSemanticsAction(label: 'Archive chat'): archive,
        if (accepted)
          const CustomSemanticsAction(label: 'Refresh chat status'): onRefresh,
      },
      excludeSemantics: true,
      button: true,
      child: InkWell(
        borderRadius: BorderRadius.circular(Radii.md),
        onTap: onTap,
        onLongPress: onLongPress,
        child: Padding(
          padding: const EdgeInsets.symmetric(
            horizontal: Grid.xxs,
            vertical: Grid.inventoryRowInset,
          ),
          child: Row(
            children: [
              SizedBox(
                width: 24,
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: Icon(BuzzIcons.messageCircle, size: 20, color: color),
                ),
              ),
              const SizedBox(width: Grid.inventoryLabelGap),
              Expanded(
                child: Text(
                  name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: navigationChatTitleTextStyle.copyWith(
                    color: color,
                    fontWeight: isUnread ? FontWeight.w600 : FontWeight.w400,
                  ),
                ),
              ),
              if (archiving) ...[
                const SizedBox(width: Grid.xs),
                const BuzzLoadingIndicator(size: 20),
              ] else if (accepted) ...[
                const SizedBox(width: Grid.xs),
                TextButton(onPressed: onRefresh, child: const Text('Refresh')),
              ] else if (working.isNotEmpty) ...[
                const SizedBox(width: Grid.xs),
                SoloWorkingDots(color: color),
              ],
            ],
          ),
        ),
      ),
    );
    if (!canArchive) return row;
    return Dismissible(
      key: ValueKey('archive-solo-$channelId'),
      // Keep the axis stable while confirmDismiss is pending. Dismissible
      // blocks further drags then; archive() also guards semantic requests.
      direction: DismissDirection.endToStart,
      movementDuration: MediaQuery.disableAnimationsOf(context)
          ? Duration.zero
          : const Duration(milliseconds: 200),
      // The channel list owns removal, only after the relay accepts the
      // archive. A rejection leaves the chat in place and ready to retry.
      confirmDismiss: (_) async {
        await archive();
        return false;
      },
      background: ExcludeSemantics(
        child: Container(
          alignment: AlignmentDirectional.centerEnd,
          padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
          decoration: BoxDecoration(
            color: context.colors.primaryContainer,
            borderRadius: BorderRadius.circular(Radii.md),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              if (archiving)
                BuzzLoadingIndicator(
                  size: 20,
                  color: context.colors.onPrimaryContainer,
                )
              else
                Icon(
                  BuzzIcons.archive,
                  color: context.colors.onPrimaryContainer,
                ),
              const SizedBox(width: Grid.xs),
              Text(
                accepted
                    ? 'Archived'
                    : archiving
                    ? 'Archiving…'
                    : 'Archive',
                style: TextStyle(color: context.colors.onPrimaryContainer),
              ),
            ],
          ),
        ),
      ),
      child: row,
    );
  }
}

// Reconnects replace the transport service while an editor may still be open.
// Retain the reviewed account/community boundary but use its current connection.
MeSync _currentMeSync(WidgetRef ref, MeSync reviewed) {
  final current = ref.read(meSyncProvider);
  if (current.community != reviewed.community ||
      current.pubkey != reviewed.pubkey) {
    throw StateError('Your account or community changed. Open Solo again.');
  }
  return current;
}

class _MeSectionEmpty extends StatelessWidget {
  const _MeSectionEmpty(this.label);
  final String label;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(
      horizontal: Grid.xxs,
      vertical: Grid.half,
    ),
    child: Align(
      alignment: Alignment.centerLeft,
      child: Text(
        label,
        style: contentListBodyTextStyle.copyWith(
          color: context.colors.onSurfaceVariant,
        ),
      ),
    ),
  );
}

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/filter_chip_bar.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import 'agent_activity_card.dart';
import 'compose_note_page.dart';
import 'note_card.dart';
import 'pulse_models.dart';
import 'pulse_provider.dart';

enum PulseTab { everyone, following, liked, agents, mine }

class PulsePage extends HookConsumerWidget {
  const PulsePage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final active = useState(PulseTab.everyone);
    final currentPubkey = ref.watch(myPubkeyProvider);
    final contactsAsync = currentPubkey == null
        ? const AsyncValue<List<ContactEntry>>.data([])
        : ref.watch(contactListProvider(currentPubkey));
    final contacts = contactsAsync.asData?.value ?? const <ContactEntry>[];
    final contactPubkeys = contacts.map((c) => c.pubkey).toList();
    final contactSet = contactPubkeys.toSet();
    final agentPubkeys =
        ref.watch(agentPubkeysProvider).asData?.value.toSet() ?? {};

    final notesAsync = switch (active.value) {
      PulseTab.everyone => ref.watch(globalNotesProvider),
      PulseTab.following => ref.watch(
        notesTimelineProvider(pulseKeyFor(contactPubkeys)),
      ),
      PulseTab.liked => ref.watch(likedNotesProvider),
      PulseTab.agents => ref.watch(agentNotesProvider),
      PulseTab.mine =>
        currentPubkey == null
            ? const AsyncValue<List<UserNote>>.data([])
            : ref.watch(notesTimelineProvider(pulseKeyFor([currentPubkey]))),
    };

    final visibleNotes = notesAsync.asData?.value ?? const <UserNote>[];
    if (visibleNotes.isNotEmpty) preloadPulseProfiles(ref, visibleNotes);
    final notesKey = pulseKeyFor(visibleNotes.map((note) => note.id));
    final reactions = ref.watch(noteReactionsProvider(notesKey));
    final reactionMap =
        reactions.asData?.value ?? const <String, PulseReactionState>{};

    return FrostedScaffold(
      resizeToAvoidBottomInset: true,
      appBar: const FrostedAppBar(title: Text('Pulse')),
      floatingActionButton: FloatingActionButton(
        heroTag: 'pulse-compose-fab',
        onPressed: () => Navigator.of(context).push(
          MaterialPageRoute<void>(builder: (_) => const ComposeNotePage()),
        ),
        tooltip: 'New note',
        shape: const CircleBorder(),
        child: const Icon(LucideIcons.plus),
      ),
      body: Column(
        children: [
          SizedBox(height: frostedAppBarHeight(context)),
          FilterChipBar<PulseTab>(
            selected: active.value,
            onSelected: (tab) => active.value = tab,
            items: const [
              FilterChipItem(
                id: PulseTab.everyone,
                label: 'Everyone',
                icon: LucideIcons.radio,
              ),
              FilterChipItem(
                id: PulseTab.following,
                label: 'Following',
                icon: LucideIcons.users,
              ),
              FilterChipItem(
                id: PulseTab.liked,
                label: 'Liked',
                icon: LucideIcons.heart,
              ),
              FilterChipItem(
                id: PulseTab.agents,
                label: 'Agents',
                icon: LucideIcons.bot,
              ),
              FilterChipItem(
                id: PulseTab.mine,
                label: 'Mine',
                icon: LucideIcons.user,
              ),
            ],
          ),
          Expanded(
            child: RefreshIndicator(
              onRefresh: () async => _refresh(ref, active.value, currentPubkey),
              child: _PulseBody(
                tab: active.value,
                notesAsync: notesAsync,
                reactions: reactionMap,
                agentPubkeys: agentPubkeys,
                contactPubkeys: contactSet,
                currentPubkey: currentPubkey,
                onReactionChanged: () =>
                    ref.invalidate(noteReactionsProvider(notesKey)),
              ),
            ),
          ),
        ],
      ),
    );
  }

  Future<void> _refresh(
    WidgetRef ref,
    PulseTab tab,
    String? currentPubkey,
  ) async {
    ref.invalidate(globalNotesProvider);
    ref.invalidate(likedNotesProvider);
    ref.invalidate(agentPubkeysProvider);
    ref.invalidate(agentNotesProvider);
    if (currentPubkey != null) {
      ref.invalidate(contactListProvider(currentPubkey));
    }
  }
}

class _PulseBody extends ConsumerWidget {
  final PulseTab tab;
  final AsyncValue<List<UserNote>> notesAsync;
  final Map<String, PulseReactionState> reactions;
  final Set<String> agentPubkeys;
  final Set<String> contactPubkeys;
  final String? currentPubkey;
  final VoidCallback onReactionChanged;

  const _PulseBody({
    required this.tab,
    required this.notesAsync,
    required this.reactions,
    required this.agentPubkeys,
    required this.contactPubkeys,
    required this.currentPubkey,
    required this.onReactionChanged,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return notesAsync.when(
      loading: () => const _TimelineSkeleton(),
      error: (_, _) => _MessageListShell(
        child: _EmptyState(
          icon: LucideIcons.circleAlert,
          message: 'Could not load Pulse. Pull to try again.',
        ),
      ),
      data: (notes) {
        if (notes.isEmpty) {
          return _MessageListShell(
            child: _EmptyState(message: _emptyMessage(tab)),
          );
        }
        if (tab == PulseTab.agents) {
          final groups = groupAgentNotes(notes);
          return ListView.separated(
            physics: const AlwaysScrollableScrollPhysics(),
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              Grid.xxs,
              Grid.gutter,
              Grid.xs,
            ),
            itemCount: groups.length,
            separatorBuilder: (_, _) => const _NoteDivider(),
            itemBuilder: (context, index) => AgentActivityCard(
              group: groups[index],
              reactions: reactions,
              onReactionChanged: onReactionChanged,
            ),
          );
        }
        return ListView.separated(
          physics: const AlwaysScrollableScrollPhysics(),
          padding: const EdgeInsets.fromLTRB(
            Grid.gutter,
            Grid.xxs,
            Grid.gutter,
            Grid.xs,
          ),
          itemCount: notes.length,
          separatorBuilder: (_, _) => const _NoteDivider(),
          itemBuilder: (context, index) {
            final note = notes[index];
            return NoteCard(
              note: note,
              reaction:
                  reactions[note.id] ??
                  const PulseReactionState(
                    count: 0,
                    reactedByCurrentUser: false,
                  ),
              isAgent: agentPubkeys.contains(note.pubkey),
              isFollowing: contactPubkeys.contains(note.pubkey),
              canFollow:
                  currentPubkey != null &&
                  currentPubkey!.toLowerCase() != note.pubkey.toLowerCase(),
              onReactionChanged: onReactionChanged,
              onFollowChanged: (_) {
                if (currentPubkey != null) {
                  ref.invalidate(contactListProvider(currentPubkey!));
                }
              },
            );
          },
        );
      },
    );
  }

  String _emptyMessage(PulseTab tab) => switch (tab) {
    PulseTab.everyone => 'No notes yet. Be the first pulse.',
    PulseTab.following => 'Follow people to build your Pulse timeline.',
    PulseTab.liked => 'Heart notes to save them here.',
    PulseTab.agents =>
      'No agent notes yet. Agents post here when they publish.',
    PulseTab.mine => 'Your notes will show up here.',
  };
}

class _MessageListShell extends StatelessWidget {
  final Widget child;

  const _MessageListShell({required this.child});

  @override
  Widget build(BuildContext context) {
    return ListView(
      physics: const AlwaysScrollableScrollPhysics(),
      padding: const EdgeInsets.all(Grid.xs),
      children: [SizedBox(height: 260, child: Center(child: child))],
    );
  }
}

class _EmptyState extends StatelessWidget {
  final IconData icon;
  final String message;

  const _EmptyState({this.icon = LucideIcons.radio, required this.message});

  @override
  Widget build(BuildContext context) {
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.all(Grid.lg),
      decoration: BoxDecoration(
        border: Border.all(color: context.colors.outlineVariant),
        borderRadius: BorderRadius.circular(Radii.lg),
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(icon, color: context.colors.onSurfaceVariant),
          const SizedBox(height: Grid.xs),
          Text(
            message,
            textAlign: TextAlign.center,
            style: context.textTheme.bodyMedium?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ],
      ),
    );
  }
}

class _TimelineSkeleton extends StatelessWidget {
  const _TimelineSkeleton();

  @override
  Widget build(BuildContext context) {
    return ListView.separated(
      padding: const EdgeInsets.fromLTRB(
        Grid.gutter,
        Grid.xxs,
        Grid.gutter,
        Grid.xs,
      ),
      itemCount: 5,
      separatorBuilder: (_, _) => const _NoteDivider(),
      itemBuilder: (_, _) => Padding(
        padding: const EdgeInsets.symmetric(vertical: Grid.twelve),
        child: Container(
          height: 64,
          decoration: BoxDecoration(
            color: context.colors.surfaceContainerHighest.withValues(
              alpha: 0.55,
            ),
            borderRadius: BorderRadius.circular(Radii.md),
          ),
        ),
      ),
    );
  }
}

class _NoteDivider extends StatelessWidget {
  const _NoteDivider();

  @override
  Widget build(BuildContext context) {
    return Divider(
      height: 1,
      thickness: 1,
      color: context.colors.outlineVariant.withValues(alpha: 0.5),
    );
  }
}

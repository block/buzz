import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/community/thread_policy_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';

/// Admin page choosing the active community's NIP-TP thread policy.
///
/// Open it only for community admins and owners; the relay rejects the
/// kind:9034 command from anyone else.
class ThreadPolicyPage extends HookConsumerWidget {
  /// Creates the thread policy page.
  const ThreadPolicyPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final relayUrl = ref.watch(relayConfigProvider.select((c) => c.baseUrl));
    final current = ref.watch(communityThreadPolicyProvider(relayUrl)).value;
    final pending = useState<ThreadPolicy?>(null);
    final selected = pending.value ?? current;

    Future<void> choose(ThreadPolicy policy) async {
      if (policy == selected) return;
      pending.value = policy;
      try {
        await ref.read(setThreadPolicyProvider)(policy);
      } catch (error) {
        if (!context.mounted) return;
        ScaffoldMessenger.maybeOf(context)?.showSnackBar(
          const SnackBar(content: Text('Couldn’t update thread replies.')),
        );
      } finally {
        if (context.mounted) pending.value = null;
      }
    }

    final enabled = current != null && pending.value == null;
    return FrostedScaffold(
      useUtilitySurfaceTheme: true,
      appBar: const FrostedAppBar(
        centerTitle: true,
        nativeLargeTitle: true,
        title: Text('Thread Replies'),
      ),
      body: ListView(
        padding: EdgeInsets.only(
          top: frostedAppBarHeight(context, nativeLargeTitle: true),
          bottom: Grid.xs,
        ),
        children: [
          for (final (policy, label) in const [
            (ThreadPolicy.nested, 'Nested'),
            (ThreadPolicy.flat, 'One level'),
          ])
            ListTile(
              key: ValueKey('thread-policy-${policy.name}'),
              title: Text(label),
              selected: selected == policy,
              enabled: enabled,
              trailing: selected == policy
                  ? Icon(
                      BuzzIcons.check,
                      size: 18,
                      color: context.colors.primary,
                    )
                  : null,
              onTap: () => choose(policy),
            ),
          Padding(
            padding: const EdgeInsets.symmetric(
              horizontal: Grid.gutter,
              vertical: Grid.xs,
            ),
            child: Text(
              'One level keeps every reply directly under the thread’s first '
              'message.',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

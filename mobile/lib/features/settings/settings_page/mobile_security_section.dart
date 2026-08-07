part of '../settings_page.dart';

class _MobileSecuritySection extends ConsumerWidget {
  const _MobileSecuritySection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final auth = ref.watch(authProvider).value;
    final community = auth?.community;
    if (community == null) return const SizedBox.shrink();

    final enabled =
        community.sensitiveActionPolicy == SensitiveActionPolicy.enabled;
    final capability = ref.watch(sensitiveActionAuthSupportedProvider);
    if (!enabled && capability.value != true) return const SizedBox.shrink();
    final authenticationName = sensitiveActionAuthenticationName(
      Theme.of(context).platform,
    );

    return AppListCard(
      label: 'Mobile security',
      children: [
        SwitchListTile(
          key: const Key('sensitive-action-confirmation-toggle'),
          secondary: const Icon(LucideIcons.shieldCheck),
          title: Text('Use $authenticationName'),
          subtitle: Text(
            enabled
                ? 'Required to open Buzz and approve protected actions.'
                : 'Require $authenticationName to open Buzz and approve protected actions.',
          ),
          value: enabled,
          onChanged: (value) => _changePolicy(context, ref, value),
        ),
      ],
    );
  }

  Future<void> _changePolicy(
    BuildContext context,
    WidgetRef ref,
    bool enabled,
  ) async {
    final currentPolicy = ref
        .read(authProvider)
        .value
        ?.community
        ?.sensitiveActionPolicy;
    if (enabled || currentPolicy == SensitiveActionPolicy.enabled) {
      final result = await ref
          .read(sensitiveActionAuthorizationSessionProvider)
          .authorize();
      if (!context.mounted) return;
      if (result != DeviceAuthResult.success) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(
            content: Text('Device authentication did not complete.'),
          ),
        );
        return;
      }
    }
    await ref
        .read(authProvider.notifier)
        .updateSensitiveActionPolicy(
          enabled
              ? SensitiveActionPolicy.enabled
              : SensitiveActionPolicy.disabledByUser,
        );
  }
}

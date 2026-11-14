part of '../settings_page.dart';

/// Opens a Builderlab web page outside the app. Overridable in tests.
final builderlabWebLauncherProvider = Provider<Future<bool> Function(Uri)>(
  (ref) =>
      (uri) => launchUrl(uri, mode: LaunchMode.externalApplication),
);

/// Links a public build to its Builderlab web dashboard, for the account
/// features mobile doesn't have: GitHub linking, billing and creating
/// communities. Sign-in's web session is not ephemeral, so it shares
/// Safari's cookies and Safari can reuse that Builderlab web session.
class _BuilderlabAccountSection extends ConsumerWidget {
  const _BuilderlabAccountSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return AppListCard(
      verticalPadding: Grid.twelve,
      children: [
        AppListRow(
          key: const Key('settings-builderlab-account'),
          title: 'Builderlab account',
          trailing: Icon(
            BuzzIcons.externalLink,
            size: 18,
            color: context.colors.onSurfaceVariant,
          ),
          onTap: () async {
            bool opened;
            try {
              opened = await ref.read(builderlabWebLauncherProvider)(
                HostedEnvironment.origin,
              );
            } on PlatformException {
              // launchUrl throws instead of returning false on some failures.
              opened = false;
            }
            if (opened || !context.mounted) return;
            ScaffoldMessenger.of(context).showFeedbackSnackBar(
              const SnackBar(content: Text('Couldn’t open Builderlab.')),
              kind: ToastKind.error,
            );
          },
        ),
      ],
    );
  }
}

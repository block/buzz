part of '../pairing_page.dart';

class _CommunityLinkForm extends HookConsumerWidget {
  const _CommunityLinkForm({
    required this.onOpen,
    required this.onBack,
    this.onCreate,
  });

  final ValueChanged<String>? onCreate;

  final ValueChanged<Uri> onOpen;
  final VoidCallback? onBack;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useTextEditingController();
    final error = useState<String?>(null);

    final flow = HostedEnvironment.enabled
        ? ref.watch(hostedFlowProvider)
        : null;
    useListenable(flow);
    final ownsFlow = ref.watch(hostedInlineEntryProvider);
    final busy =
        flow != null &&
        ownsFlow &&
        !flow.accountOnly &&
        (flow.stage == HostedFlowStage.working ||
            flow.stage == HostedFlowStage.signIn);
    final failed =
        flow != null && ownsFlow && !flow.accountOnly && flow.problem != null;

    void submit() {
      if (busy || (failed && !flow.canRetry)) return;
      final create = onCreate;
      if (create != null) {
        final name = controller.text.trim().toLowerCase();
        if (!HostedCreateCommunity.validName(name)) {
          error.value =
              'Use letters, numbers and single hyphens (up to 63 characters).';
          return;
        }
        error.value = null;
        FocusManager.instance.primaryFocus?.unfocus();
        create(name);
        return;
      }
      final link = parseCommunityLinkInput(controller.text);
      if (link == null) {
        error.value = 'Enter a valid community address or invite link.';
        return;
      }
      error.value = null;
      ref.read(hostedInlineEntryProvider.notifier).setActive(true);
      onOpen(link);
    }

    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        TextField(
          key: Key(
            onCreate != null
                ? 'community-create-input'
                : 'community-link-input',
          ),
          controller: controller,
          enabled: !busy,
          keyboardType: TextInputType.url,
          textInputAction: TextInputAction.go,
          autocorrect: false,
          enableSuggestions: false,
          style: context.textTheme.bodyMedium?.copyWith(
            color: context._onboardingInk,
          ),
          cursorColor: context._onboardingInk,
          decoration: InputDecoration(
            labelText: onCreate != null
                ? 'Community address'
                : 'Community link or address',
            helperText: onCreate != null
                ? '.${HostedEnvironment.communitySuffix}'
                : null,
            helperStyle: TextStyle(color: context._onboardingMutedInk),
            labelStyle: TextStyle(color: context._onboardingInk),
            hintText: onCreate != null
                ? 'my-community'
                : 'community.example.com',
            hintStyle: TextStyle(color: context._onboardingMutedInk),
            errorText:
                error.value ??
                (failed ? hostedFlowProblemMessage(flow.problem) : null),
            filled: true,
            fillColor: context._onboardingInputSurface,
            enabledBorder: context._inputBorder,
            focusedBorder: context._inputBorder.copyWith(
              borderSide: BorderSide(color: context._onboardingInk),
            ),
          ),
          onChanged: (_) {
            error.value = null;
            if (failed) unawaited(flow.leave());
          },
          onSubmitted: (_) => submit(),
        ),
        const SizedBox(height: Grid.xs),
        FilledButton(
          style: context._onboardingGlassButtonStyle,
          onPressed: busy || (failed && !flow.canRetry) ? null : submit,
          child: Text(busy ? 'Connecting…' : 'Continue'),
        ),
        const SizedBox(height: Grid.xxs),
        TextButton(
          style: context._onboardingGhostButtonStyle,
          onPressed: () {
            unawaited(flow?.leave());
            onBack?.call();
          },
          child: const Text('Back'),
        ),
      ],
    );
  }
}

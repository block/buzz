part of '../pairing_page.dart';

class _CommunityLinkForm extends HookConsumerWidget {
  const _CommunityLinkForm({
    required this.onOpen,
    required this.onBack,
    this.onCreate,
    this.controller,
    this.focusNode,
    this.showBack = true,
    this.fullPage = false,
    this.autofocus = true,
    this.showHeading = true,
    this.layoutBuilder,
  });

  final ValueChanged<String>? onCreate;
  final TextEditingController? controller;
  final FocusNode? focusNode;
  final bool showBack;
  final bool fullPage;
  final bool autofocus;
  final bool showHeading;
  final Widget Function(Widget fields, Widget action)? layoutBuilder;

  final ValueChanged<Uri> onOpen;
  final VoidCallback? onBack;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final localController = useTextEditingController();
    final controller = this.controller ?? localController;
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

    void changed(String _) {
      error.value = null;
      if (failed) unawaited(flow.leave());
    }

    final problem =
        error.value ?? (failed ? hostedFlowProblemMessage(flow.problem) : null);
    final iosInput = fullPage && defaultTargetPlatform == TargetPlatform.iOS;

    final fields = <Widget>[
      if (fullPage && showHeading) ...[
        Text(
          'Enter a community link',
          style: context.textTheme.headlineSmall?.copyWith(
            fontWeight: FontWeight.w600,
          ),
        ),
        const SizedBox(height: Grid.xs),
        Text(
          'Paste an invite link or community address.',
          style: context.textTheme.titleLarge?.copyWith(
            color: context.colors.onSurfaceVariant,
          ),
        ),
        const SizedBox(height: Grid.lg),
      ],
      if (iosInput) ...[
        CupertinoTextField(
          key: const Key('community-link-input'),
          controller: controller,
          focusNode: focusNode,
          autofocus: fullPage && autofocus,
          enabled: !busy,
          placeholder: 'Community link or address',
          placeholderStyle: context.textTheme.titleLarge?.copyWith(
            color: CupertinoColors.placeholderText.resolveFrom(context),
          ),
          keyboardType: TextInputType.url,
          textInputAction: TextInputAction.go,
          autocorrect: false,
          enableSuggestions: false,
          padding: const EdgeInsets.symmetric(vertical: Grid.xs),
          style: context.textTheme.titleLarge?.copyWith(
            color: context.colors.onSurface,
          ),
          cursorColor: context.colors.primary,
          decoration: null,
          onChanged: changed,
          onSubmitted: (_) => submit(),
        ),
        if (problem != null)
          Padding(
            padding: const EdgeInsets.only(top: Grid.xs),
            child: Text(problem, style: TextStyle(color: context.colors.error)),
          ),
      ] else
        TextField(
          key: Key(
            onCreate != null
                ? 'community-create-input'
                : 'community-link-input',
          ),
          controller: controller,
          focusNode: focusNode,
          autofocus: fullPage && autofocus,
          enabled: !busy,
          keyboardType: TextInputType.url,
          textInputAction: TextInputAction.go,
          autocorrect: false,
          enableSuggestions: false,
          style:
              (fullPage
                      ? context.textTheme.titleLarge
                      : context.textTheme.bodyMedium)
                  ?.copyWith(color: context._onboardingInk),
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
          onChanged: changed,
          onSubmitted: (_) => submit(),
        ),
    ];
    final action = FilledButton(
      style: fullPage
          ? FilledButton.styleFrom(
              backgroundColor: layoutBuilder == null
                  ? context.colors.primary
                  : Colors.transparent,
              shadowColor: Colors.transparent,
              animationDuration: Duration.zero,
              foregroundColor: context.colors.onPrimary,
              minimumSize: const Size.fromHeight(52),
              shape: const StadiumBorder(),
            )
          : context._onboardingGlassButtonStyle,
      onPressed: busy || (failed && !flow.canRetry) ? null : submit,
      child: Text(busy ? 'Connecting…' : 'Continue'),
    );
    if (layoutBuilder != null) {
      return layoutBuilder!(
        SingleChildScrollView(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: fields,
          ),
        ),
        action,
      );
    }
    return Column(
      mainAxisSize: fullPage ? MainAxisSize.max : MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (fullPage)
          Expanded(
            child: SingleChildScrollView(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: fields,
              ),
            ),
          )
        else
          ...fields,
        const SizedBox(height: Grid.xs),
        action,
        const SizedBox(height: Grid.xxs),
        if (showBack)
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

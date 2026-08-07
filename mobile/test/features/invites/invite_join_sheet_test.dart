import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'package:buzz/features/invites/invite_join_provider.dart';
import 'package:buzz/features/invites/invite_join_sheet.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/theme/theme.dart';

void main() {
  testWidgets('policy confirmations start unticked and gate acceptance', (
    tester,
  ) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [inviteJoinProvider.overrideWith(_PolicyReviewNotifier.new)],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: const Scaffold(body: InviteJoinSheet()),
        ),
      ),
    );

    expect(find.text('Terms of Service'), findsOneWidget);
    expect(find.text('Be kind to other members.'), findsOneWidget);
    expect(find.text('Privacy Policy'), findsOneWidget);
    expect(find.text('We retain your messages.'), findsOneWidget);

    var checkboxes = tester
        .widgetList<Checkbox>(find.byType(Checkbox))
        .toList();
    expect(checkboxes, hasLength(2));
    expect(checkboxes.every((checkbox) => checkbox.value == false), isTrue);
    expect(
      tester
          .widget<FilledButton>(find.byKey(const Key('accept-join-policy')))
          .onPressed,
      isNull,
    );

    final ageConfirmation = find.text('I am 18 years of age or older.');
    final agreementConfirmation = find.text(
      'I agree to the Terms of Service and Privacy Policy shown above.',
    );
    await tester.ensureVisible(ageConfirmation);
    await tester.tap(ageConfirmation);
    await tester.pump();
    await tester.ensureVisible(agreementConfirmation);
    await tester.tap(agreementConfirmation);
    await tester.pump();

    checkboxes = tester.widgetList<Checkbox>(find.byType(Checkbox)).toList();
    expect(checkboxes.every((checkbox) => checkbox.value == true), isTrue);
    expect(
      tester
          .widget<FilledButton>(find.byKey(const Key('accept-join-policy')))
          .onPressed,
      isNotNull,
    );
  });
}

class _PolicyReviewNotifier extends InviteJoinNotifier {
  @override
  InviteJoinState build() => const InviteJoinState(
    status: InviteJoinStatus.reviewingPolicy,
    invite: InviteDeepLink(relayUrl: 'wss://relay.example.com', code: 'code'),
    host: 'relay.example.com',
    communityName: 'relay.example.com',
    policy: InviteJoinPolicy(
      termsMarkdown: 'Be kind to other members.',
      privacyMarkdown: 'We retain your messages.',
      ageAttestationRequired: true,
      version: 'policy-v1',
    ),
  );
}

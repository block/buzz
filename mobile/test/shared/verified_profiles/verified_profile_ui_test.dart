import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/verified_profiles/verified_profile.dart';
import 'package:buzz/shared/verified_profiles/verified_profile_directory.dart';
import 'package:buzz/shared/widgets/message_author_meta.dart';
import 'package:buzz/shared/widgets/verified_mark.dart';
import 'package:buzz/shared/widgets/verified_profile_details.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _Directory extends VerifiedProfilesNotifier {
  _Directory(this.initial);
  final Map<String, VerifiedProfile> initial;

  @override
  Map<String, VerifiedProfile> build() => initial;

  void replace(Map<String, VerifiedProfile> next) => state = next;
}

VerifiedProfile record({bool checked = true}) => VerifiedProfile(
  displayName: 'Ada Lovelace',
  username: 'ada',
  fullName: 'Augusta Ada King',
  issuerLabel: 'Example Corp',
  verifiedAt: DateTime(2026, 1, 1).millisecondsSinceEpoch ~/ 1000,
  checked: checked,
  multipleIssuers: false,
);

void main() {
  final ada = 'a' * 64;

  testWidgets('mark and details show the issuer and a historical time', (
    tester,
  ) async {
    final directory = _Directory({ada: record()});
    await tester.pumpWidget(
      ProviderScope(
        overrides: [verifiedProfilesProvider.overrideWith(() => directory)],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: MediaQuery(
              data: const MediaQueryData(textScaler: TextScaler.linear(2)),
              child: Column(
                children: [
                  MessageAuthorMeta(
                    displayName: 'Ada Lovelace',
                    timestamp: '9:41',
                    nameColor: Colors.black,
                    metadataColor: Colors.grey,
                    mark: VerifiedMark(pubkey: ada),
                  ),
                  VerifiedProfileDetails(
                    pubkey: ada,
                    now: DateTime(2026, 1, 3),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
    expect(find.byKey(const ValueKey('verified-mark')), findsOneWidget);
    expect(find.byTooltip('Verified by Example Corp'), findsOneWidget);
    expect(find.text('Augusta Ada King'), findsOneWidget);
    expect(find.text('Verified by Example Corp'), findsOneWidget);
    expect(find.text('Last verified 2 days ago'), findsOneWidget);

    directory.replace({ada: record(checked: false)});
    await tester.pump();
    expect(find.text('Last verified 2 days ago (cached)'), findsOneWidget);

    // Hidden discovery or a retraction removes every trace.
    directory.replace({});
    await tester.pump();
    expect(find.byKey(const ValueKey('verified-mark')), findsNothing);
    expect(
      find.byKey(const ValueKey('verified-profile-details')),
      findsNothing,
    );
  });
}

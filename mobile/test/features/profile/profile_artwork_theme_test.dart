import 'package:buzz/features/profile/profile_artwork_theme.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  for (final rgb in [0xffeee3b8, 0xff20132b, 0xff9665bc]) {
    testWidgets('iOS adapts text to bottom-edge colour $rgb', (tester) async {
      final url = Uri.dataFromString(
        '<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32">'
        '<rect width="32" height="32" fill="#ff0000"/>'
        '<rect y="25" width="32" height="7" fill="#${rgb.toRadixString(16).substring(2)}"/></svg>',
        mimeType: 'image/svg+xml',
      ).toString();
      late ThemeData profile;
      await tester.runAsync(() async {
        await tester.pumpWidget(
          ProviderScope(
            child: MaterialApp(
              theme: AppTheme.light().copyWith(platform: TargetPlatform.iOS),
              home: HookConsumer(
                builder: (context, ref, _) {
                  profile = useProfileArtworkTheme(context, ref, url);
                  return const SizedBox();
                },
              ),
            ),
          ),
        );
        final deadline = DateTime.now().add(const Duration(seconds: 5));
        while (profile.scaffoldBackgroundColor ==
                utilitySurfaceThemeData(
                  AppTheme.light(),
                ).scaffoldBackgroundColor &&
            DateTime.now().isBefore(deadline)) {
          await tester.binding.endOfFrame.timeout(
            const Duration(milliseconds: 20),
            onTimeout: () {},
          );
          await tester.pump();
        }
        final actual = profile.scaffoldBackgroundColor;
        final expected = Color(rgb);
        // Resampling the SVG can mix a few pixels at the band boundary.
        expect(actual.r, closeTo(expected.r, 4 / 255));
        expect(actual.g, closeTo(expected.g, 4 / 255));
        expect(actual.b, closeTo(expected.b, 4 / 255));
        expect(profile.colorScheme.surface, actual);
        final ink = profile.colorScheme.onSurface;
        if (rgb == 0xffeee3b8) {
          expect(ink, Colors.black);
          final body = profile.colorScheme.onSurfaceVariant;
          expect(body, profileDarkInk(actual));
          expect(body, isNot(Colors.black));
          expect(
            (actual.computeLuminance() + .05) / (body.computeLuminance() + .05),
            greaterThanOrEqualTo(4.5),
          );
        } else {
          expect(ink, Colors.white);
        }
        expect(
          profile.brightness,
          rgb == 0xffeee3b8 ? Brightness.light : Brightness.dark,
        );
      });
    });
  }
}

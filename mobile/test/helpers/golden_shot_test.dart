// Self-test for the golden-screenshot helper.
//
// This deliberately does NOT compare against a committed golden PNG: binary
// goldens in the repo would need refreshing on every font or engine bump, and
// they cannot tell you *why* a shot broke. Instead it asserts the two things
// the helper exists to guarantee — that the text font and the package-scoped
// icon font actually rasterise — by rendering pairs of inputs that are
// indistinguishable when the fonts are missing and obviously different when
// they are present.
//
// It passes under a plain `flutter test`; no `--update-goldens` needed.

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import 'golden_shot.dart';

const Key _shotKey = ValueKey<String>('golden-shot-self-test');

/// A fixed-size white swatch with a repaint boundary, so every capture in this
/// file is the same geometry and only the glyphs differ.
Widget _swatch(Widget child) {
  return MaterialApp(
    debugShowCheckedModeBanner: false,
    home: Scaffold(
      backgroundColor: Colors.white,
      body: Center(
        child: RepaintBoundary(
          key: _shotKey,
          child: Container(
            width: 200,
            height: 90,
            color: Colors.white,
            alignment: Alignment.center,
            child: child,
          ),
        ),
      ),
    ),
  );
}

Future<Uint8List> _render(WidgetTester tester, Widget child) async {
  await tester.pumpWidget(_swatch(child));
  await tester.pumpAndSettle();
  return rasterize(tester, find.byKey(_shotKey));
}

/// Accepts every comparison so `captureShot` can be exercised end to end
/// without a PNG on disk.
class _AcceptAllGoldens extends GoldenFileComparator {
  @override
  Future<bool> compare(Uint8List imageBytes, Uri golden) async => true;

  @override
  Future<void> update(Uri golden, Uint8List imageBytes) async {}
}

/// Accepts every comparison too, but only after doing the same **real**
/// `dart:io` work `LocalFileComparator` does — the part an in-memory stub
/// silently skips.
///
/// A real I/O future never completes inside the fake-async zone a widget test
/// runs in, so this comparator hangs forever unless `captureShot` awaits the
/// golden comparison inside `runAsync`. That is the whole point of it: the
/// accept-all stub above completes synchronously and cannot tell you whether
/// the real workflow works at all.
class _RealIoGoldens extends GoldenFileComparator {
  _RealIoGoldens(this.baseDir);

  final Directory baseDir;

  File fileFor(Uri golden) => File(
    '${baseDir.path}${Platform.pathSeparator}'
    '${golden.path.replaceAll('/', Platform.pathSeparator)}',
  );

  @override
  Future<bool> compare(Uint8List imageBytes, Uri golden) async {
    await update(golden, imageBytes);
    return true;
  }

  @override
  Future<void> update(Uri golden, Uint8List imageBytes) async {
    final file = fileFor(golden);
    await file.parent.create(recursive: true);
    await file.writeAsBytes(imageBytes, flush: true);
  }
}

void main() {
  setUp(() async {
    // Idempotent: the first test pays for the font registration, the rest are
    // free. Calling it per test is the pattern the README tells contributors
    // to follow, so the self-test follows it too.
    await loadAppFonts();
  });

  test('loadAppFonts registers the package-qualified icon families', () {
    // The trap this guards: LucideIcons' IconData carries
    // fontPackage: 'lucide_icons_flutter', so the family Flutter looks up is
    // 'packages/lucide_icons_flutter/Lucide'. Registering bare 'Lucide'
    // silently leaves every icon as a tofu box.
    expect(
      registeredFontFamilies,
      contains('packages/lucide_icons_flutter/Lucide'),
    );
    expect(
      registeredFontFamilies.where((family) => family.contains('Lucide')),
      everyElement(startsWith('packages/lucide_icons_flutter/')),
    );
    for (final weight in const <int>[100, 200, 300, 400, 500, 600]) {
      expect(
        registeredFontFamilies,
        contains('packages/lucide_icons_flutter/Lucide$weight'),
        reason: 'The Lucide$weight weight variant was never registered.',
      );
    }
    expect(registeredFontFamilies, contains('Inter'));
    expect(registeredFontFamilies, contains('MaterialIcons'));
  });

  testWidgets('setPhoneSurface sizes the view to a phone', (tester) async {
    final defaultSize = tester.view.physicalSize;

    setPhoneSurface(tester);
    expect(tester.view.physicalSize, isNot(defaultSize));
    expect(tester.view.devicePixelRatio, kPhoneDevicePixelRatio);
    expect(
      tester.view.physicalSize,
      Size(
        kPhoneLogicalSize.width * kPhoneDevicePixelRatio,
        kPhoneLogicalSize.height * kPhoneDevicePixelRatio,
      ),
    );

    await tester.pumpWidget(
      const MaterialApp(home: Scaffold(body: SizedBox.expand())),
    );
    // The app really lays out at phone logical size, not the 800x600 default.
    expect(tester.getSize(find.byType(MaterialApp)), kPhoneLogicalSize);
    // setPhoneSurface registers its own addTearDown to undo all of the above,
    // which is why it is safe to call once per test.
  });

  testWidgets('text renders with real glyphs, not tofu boxes', (tester) async {
    setPhoneSurface(tester);

    const style = TextStyle(
      fontFamily: 'Inter',
      fontSize: 34,
      color: Colors.black,
    );
    final wide = await _render(tester, const Text('MMMM', style: style));
    final narrow = await _render(tester, const Text('llll', style: style));

    // Every glyph in the stub test font is an identical filled square, so
    // 'MMMM' and 'llll' rasterise identically when no real font is loaded.
    expect(
      wide,
      isNot(equals(narrow)),
      reason:
          'Identical captures for different glyphs means the Inter font never '
          'registered and text is rendering as tofu boxes.',
    );

    // Antialiased curves produce many intermediate greys; a blank surface or a
    // run of solid tofu boxes produces a handful of flat colours.
    await _render(tester, const Text('Buzz', style: style));
    final shades = await distinctColorCount(tester, find.byKey(_shotKey));
    expect(shades, greaterThan(8), reason: 'Capture has $shades colours.');
    expect(await isVisuallyBlank(tester, find.byKey(_shotKey)), isFalse);
  });

  testWidgets('lucide icons render with real glyphs', (tester) async {
    setPhoneSurface(tester);

    const size = 56.0;
    final bell = await _render(
      tester,
      const Icon(LucideIcons.bell, size: size, color: Colors.black),
    );
    final house = await _render(
      tester,
      const Icon(LucideIcons.house, size: size, color: Colors.black),
    );
    final weighted = await _render(
      tester,
      const Icon(LucideIcons.bell100, size: size, color: Colors.black),
    );

    // Unregistered icon fonts collapse every codepoint to the same tofu box.
    expect(
      bell,
      isNot(equals(house)),
      reason:
          'Two different Lucide icons rasterised identically — the icon font '
          'family is almost certainly registered without its '
          'packages/lucide_icons_flutter/ prefix.',
    );
    expect(
      bell,
      isNot(equals(weighted)),
      reason: 'LucideIcons.bell100 rasterised the same bytes as bell.',
    );

    // The tree still holds the weighted render. Byte inequality alone is NOT
    // enough for the weight variants: an unregistered Lucide100 draws a tofu
    // box, and a box differs from a real bell just as much as a thin bell does.
    // Only the shade count separates them — a solid box is 2 flat colours, an
    // antialiased glyph is well over a dozen.
    final weightedShades = await distinctColorCount(
      tester,
      find.byKey(_shotKey),
    );
    expect(
      weightedShades,
      greaterThan(8),
      reason:
          'LucideIcons.bell100 rasterised to $weightedShades colours — that is '
          'a tofu box, so packages/lucide_icons_flutter/Lucide100 never '
          'registered.',
    );

    expect(await isVisuallyBlank(tester, find.byKey(_shotKey)), isFalse);
  });

  testWidgets('captureShot rejects byte-identical shots', (tester) async {
    final previousComparator = goldenFileComparator;
    goldenFileComparator = _AcceptAllGoldens();
    resetShotDigests();
    addTearDown(() {
      goldenFileComparator = previousComparator;
      resetShotDigests();
    });

    setPhoneSurface(tester);
    await tester.pumpWidget(
      _swatch(
        const Text(
          'Buzz',
          style: TextStyle(
            fontFamily: 'Inter',
            fontSize: 28,
            color: Colors.black,
          ),
        ),
      ),
    );

    // First shot goes through the full matchesGoldenFile path.
    await captureShot(tester, find.byKey(_shotKey), '01-first');

    // Re-capturing the unchanged tree under a new name is the classic
    // "two screenshots of the same state" bug, and the helper must catch it.
    await expectLater(
      () => captureShot(tester, find.byKey(_shotKey), '02-duplicate'),
      throwsA(
        isA<StateError>().having(
          (error) => error.message,
          'message',
          contains('byte-identical'),
        ),
      ),
    );

    // ...and the escape hatch still works for genuinely identical states.
    await captureShot(
      tester,
      find.byKey(_shotKey),
      '03-duplicate-allowed',
      failOnDuplicate: false,
    );
  });

  testWidgets(
    'captureShot survives a comparator that does real file I/O',
    (tester) async {
      // Regression guard for a bug this file used to be blind to. Handed raw
      // bytes, matchesGoldenFile awaits the golden comparator OUTSIDE runAsync,
      // and the real LocalFileComparator reads and writes with dart:io futures
      // that never complete inside a widget test's fake-async zone — so
      // `flutter test --update-goldens` hung forever instead of writing a PNG,
      // while every assertion here stayed green against the in-memory stub.
      //
      // The explicit timeout matters as much as the assertions: flutter_test
      // disables per-test timeouts by default, so a regression would otherwise
      // hang CI indefinitely rather than fail it.
      final tempDir = Directory.systemTemp.createTempSync(
        'golden_shot_selftest',
      );
      final previousComparator = goldenFileComparator;
      final comparator = _RealIoGoldens(tempDir);
      goldenFileComparator = comparator;
      resetShotDigests();
      addTearDown(() {
        goldenFileComparator = previousComparator;
        resetShotDigests();
        tempDir.deleteSync(recursive: true);
      });

      setPhoneSurface(tester);
      await tester.pumpWidget(
        _swatch(
          const Text(
            'Buzz',
            style: TextStyle(
              fontFamily: 'Inter',
              fontSize: 28,
              color: Colors.black,
            ),
          ),
        ),
      );

      await captureShot(tester, find.byKey(_shotKey), 'real-comparator');

      final written = comparator.fileFor(
        Uri.parse('goldens/real-comparator.png'),
      );
      expect(
        written.existsSync(),
        isTrue,
        reason: 'captureShot produced no PNG at ${written.path}',
      );
      // PNG magic number — proves a real image landed, not a truncated write.
      expect(written.readAsBytesSync().take(4), <int>[0x89, 0x50, 0x4e, 0x47]);
      expect(written.lengthSync(), greaterThan(500));
    },
    timeout: const Timeout(Duration(seconds: 60)),
  );
}

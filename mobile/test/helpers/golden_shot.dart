/// Golden-screenshot helpers for the Buzz mobile widget tests.
///
/// The point of this file is to make it possible to produce PR-quality mobile
/// screenshots from a plain `flutter test` run — no Mac, no simulator, no
/// device farm. A widget test rendered at a phone surface with the real app
/// fonts loaded is visually indistinguishable from a simulator capture for
/// review purposes, and it runs on every contributor's machine including
/// Windows.
///
/// Typical use:
///
/// ```dart
/// testWidgets('channel list', (tester) async {
///   await loadAppFonts();
///   setPhoneSurface(tester);
///   await tester.pumpWidget(WidgetHelpers.testable(child: const MyPage()));
///   await captureShot(tester, find.byType(MyPage), '01-channel-list');
/// });
/// ```
///
/// Then generate/refresh the PNGs with:
///
/// ```sh
/// flutter test --update-goldens test/features/my_feature
/// ```
///
/// The PNGs land in a `goldens/` directory next to the test file and can be
/// posted straight to a PR (see `scripts/post-screenshots.sh` at the repo
/// root).
///
/// See `test/helpers/README.md` for the full runbook and the font traps this
/// file exists to encode.
library;

import 'dart:io';
import 'dart:isolate';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

/// Logical size of the default capture surface (iPhone 14 / 15 class device).
const Size kPhoneLogicalSize = Size(390, 844);

/// Device pixel ratio of the default capture surface.
const double kPhoneDevicePixelRatio = 2.0;

/// The font family the app's text theme asks for.
const String _kAppFontFamily = 'Inter';

/// The monospace family used by code blocks in message bodies.
const String _kMonoFontFamily = 'GeistMono';

/// The pub package that ships the icon font used across the app.
const String _kIconPackage = 'lucide_icons_flutter';

/// Weight variants shipped by the icon package, as `Lucide<weight>` families.
const List<int> _kIconWeights = <int>[100, 200, 300, 400, 500, 600];

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------

bool _fontsLoaded = false;

/// Registers the real app fonts with the test font collection.
///
/// `flutter_test` ships with a single stub font ("Ahem") that draws every
/// glyph as a filled box, so **any** widget-test screenshot is unreadable tofu
/// until the real fonts are registered. Call this at the top of every test
/// that captures a shot. It is idempotent and cheap after the first call.
///
/// What gets registered, and why each one matters:
///
/// * `Inter` — the app's text family (`lib/shared/theme/text_theme.dart`).
///   Without it every string renders as boxes.
/// * `GeistMono` — the monospace family used for code.
/// * `packages/lucide_icons_flutter/Lucide` (plus the `Lucide100`..`Lucide600`
///   weight variants) — the icon font. Note the `packages/<pkg>/` prefix: the
///   `IconData` constants in `lucide_icons_flutter` carry
///   `fontPackage: 'lucide_icons_flutter'`, and Flutter resolves that to the
///   family name `packages/lucide_icons_flutter/Lucide`. Registering the bare
///   name `Lucide` "succeeds" and still leaves **every icon in the app** as a
///   tofu box, which is the easiest way there is to ship a broken screenshot.
/// * `MaterialIcons` and `Roboto` — shipped by the Flutter SDK and used by
///   framework widgets (back buttons, dropdown chevrons, unstyled text).
///
/// Throws a [StateError] with an actionable message if a font file cannot be
/// located. It never hangs: every font file is read **synchronously** before
/// being handed to [FontLoader]. `FontLoader.load()` awaits the futures it was
/// given, and a future that completes with an error there wedges the test run
/// forever rather than failing it — so the read must not be async.
Future<void> loadAppFonts() async {
  if (_fontsLoaded) {
    return;
  }

  final appFonts = '${mobilePackageRoot()}/assets/fonts';
  await _registerFont(_kAppFontFamily, <String>[
    '$appFonts/InterVariable.ttf',
    '$appFonts/InterVariable-Italic.ttf',
  ]);
  await _registerFont(_kMonoFontFamily, <String>[
    '$appFonts/GeistMono-Variable.ttf',
    '$appFonts/GeistMono-Italic-Variable.ttf',
  ]);

  // The family names MUST carry the `packages/<pkg>/` prefix — see above.
  // `Lucide` is the fixed-weight font; `Lucide100`..`Lucide600` are the
  // variable-weight builds behind `LucideIcons.<name><weight>`.
  final iconRoot = iconPackageRoot();
  await _registerFont('packages/$_kIconPackage/Lucide', <String>[
    '$iconRoot/assets/lucide.ttf',
  ]);
  for (final weight in _kIconWeights) {
    await _registerFont('packages/$_kIconPackage/Lucide$weight', <String>[
      '$iconRoot/assets/build_font/LucideVariable-w$weight.ttf',
    ]);
  }

  final sdkFonts = '${flutterSdkRoot()}/bin/cache/artifacts/material_fonts';
  await _registerFont('MaterialIcons', <String>[
    '$sdkFonts/materialicons-regular.otf',
  ]);
  await _registerFont('Roboto', <String>[
    '$sdkFonts/roboto-regular.ttf',
    '$sdkFonts/roboto-medium.ttf',
    '$sdkFonts/roboto-bold.ttf',
  ]);

  _fontsLoaded = true;
}

/// Family names [loadAppFonts] has actually registered, in registration order.
///
/// Recorded by [_registerFont] as each family goes in, rather than restated as
/// a literal list: a hardcoded list keeps happily reporting families that a
/// later edit stopped registering, which is precisely the silent breakage this
/// helper exists to catch. Empty until [loadAppFonts] has been awaited.
List<String> get registeredFontFamilies =>
    List<String>.unmodifiable(_registeredFamilies);

final List<String> _registeredFamilies = <String>[];

Future<void> _registerFont(String family, List<String> paths) async {
  final loader = FontLoader(family);
  for (final path in paths) {
    // Synchronous read on purpose: see loadAppFonts' doc comment.
    loader.addFont(Future<ByteData>.value(_readFontSync(path, family)));
  }
  await loader.load();
  if (!_registeredFamilies.contains(family)) {
    _registeredFamilies.add(family);
  }
}

ByteData _readFontSync(String path, String family) {
  final file = File(path);
  if (!file.existsSync()) {
    throw StateError(
      'golden_shot: cannot register font family "$family" — file not found:\n'
      '  $path\n'
      'If this is a pub package font, run `flutter pub get` in mobile/ and '
      'retry. If it is an SDK font, check that FLUTTER_ROOT points at a '
      'Flutter checkout with a populated bin/cache/artifacts.',
    );
  }
  return ByteData.sublistView(file.readAsBytesSync());
}

// ---------------------------------------------------------------------------
// Surface
// ---------------------------------------------------------------------------

/// Sizes the test view to a phone surface so captures look like a real device.
///
/// The default `flutter_test` surface is 800x600 at DPR 1 — a squat landscape
/// tablet that makes every mobile layout look wrong. This sets a portrait
/// phone surface, pins the text scale factor to 1.0 so host accessibility
/// settings cannot drift the output, and zeroes the safe-area padding so shots
/// are not offset by a simulated notch.
///
/// Everything it changes is restored via `addTearDown`, so it is safe to call
/// once per test.
void setPhoneSurface(
  WidgetTester tester, {
  Size logical = kPhoneLogicalSize,
  double dpr = kPhoneDevicePixelRatio,
}) {
  final view = tester.view;

  view.devicePixelRatio = dpr;
  view.physicalSize = Size(logical.width * dpr, logical.height * dpr);
  view.padding = FakeViewPadding.zero;
  view.viewPadding = FakeViewPadding.zero;
  view.viewInsets = FakeViewPadding.zero;
  tester.platformDispatcher.textScaleFactorTestValue = 1.0;

  addTearDown(() {
    view.resetDevicePixelRatio();
    view.resetPhysicalSize();
    view.resetPadding();
    view.resetViewPadding();
    view.resetViewInsets();
    tester.platformDispatcher.clearTextScaleFactorTestValue();
  });
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

/// Digest -> shot name, so identical captures inside one run are caught.
final Map<int, String> _shotDigests = <int, String>{};

/// Renders [finder] and compares it against `goldens/<name>.png`.
///
/// Run `flutter test --update-goldens <path>` to write the PNGs, then commit
/// or post them. A subsequent plain `flutter test` compares against what was
/// written, which is what keeps the screenshots honest over time.
///
/// [name] should sort in the order you want the shots to appear in the PR
/// comment, e.g. `01-empty-state`, `02-with-messages`.
///
/// If two shots in the same run produce byte-identical images this throws by
/// default. That is almost never a coincidence: it means both captures
/// rendered the *same* state — the most common screenshot bug there is, and
/// one that is easy to miss when eyeballing a wall of thumbnails. Fix the test
/// rather than silencing it; pass `failOnDuplicate: false` only when two
/// states are genuinely expected to be pixel-identical.
Future<void> captureShot(
  WidgetTester tester,
  Finder finder,
  String name, {
  String goldenDir = 'goldens',
  bool failOnDuplicate = true,
  bool settle = true,
}) async {
  if (settle) {
    await tester.pumpAndSettle();
  }

  final png = await rasterize(tester, finder);

  final digest = _digest(png);
  final previous = _shotDigests[digest];
  if (previous != null && previous != name) {
    if (failOnDuplicate) {
      throw StateError(
        'golden_shot: shot "$name" is byte-identical to "$previous".\n'
        'Two captures rendered the same state. Check that the test actually '
        'drove the UI into a different state between the two shots, and that '
        'the finder is scoped to the subject rather than the whole page. Pass '
        'failOnDuplicate: false if they are genuinely meant to match.',
      );
    }
    debugPrint(
      'golden_shot: warning — shot "$name" is byte-identical to "$previous".',
    );
  }
  _shotDigests[digest] = name;

  // The golden comparison MUST run inside runAsync, and that is not a
  // stylistic choice. When the value handed to matchesGoldenFile is a
  // List<int>, flutter_test's matcher awaits the golden comparator *outside*
  // runAsync — see the early byte path in MatchesGoldenFile.matchAsync
  // (flutter_test/lib/src/_matchers_io.dart). LocalFileComparator reads and
  // writes with real dart:io futures, and a real I/O future never completes
  // inside the fake-async zone a widget test runs in. Without this wrapper,
  // `flutter test --update-goldens` hangs forever with no output instead of
  // writing a PNG, and nothing surfaces the cause: a stubbed in-memory
  // comparator completes synchronously and looks perfectly healthy.
  await TestWidgetsFlutterBinding.instance.runAsync(
    () => expectLater(png, matchesGoldenFile('$goldenDir/$name.png')),
  );
}

/// Renders [finder]'s nearest repaint boundary and returns encoded PNG bytes.
///
/// Useful on its own for assertions about what was drawn (see
/// [isVisuallyBlank] and [distinctColorCount]) without involving a golden file
/// at all.
Future<Uint8List> rasterize(WidgetTester tester, Finder finder) {
  return _withCapturedImage(finder, (image) async {
    final data = await image.toByteData(format: ui.ImageByteFormat.png);
    if (data == null) {
      throw StateError('golden_shot: could not encode the captured image.');
    }
    return Uint8List.fromList(data.buffer.asUint8List());
  });
}

/// Whether every pixel of [finder]'s capture is the same colour.
///
/// A uniform capture is the signature of a render that produced nothing — most
/// often because the fonts were never loaded and the widget under test is pure
/// text, or because the finder resolved to an offstage subtree. Use it to
/// assert that a shot has actual content in it.
Future<bool> isVisuallyBlank(WidgetTester tester, Finder finder) {
  return _withCapturedImage(finder, (image) async {
    final pixels = await _rawPixels(image);
    if (pixels.isEmpty) {
      return true;
    }
    final first = pixels.first;
    for (final pixel in pixels) {
      if (pixel != first) {
        return false;
      }
    }
    return true;
  });
}

/// Number of distinct colours in [finder]'s capture.
///
/// A crude but effective "did the glyphs actually rasterise" probe: antialiased
/// text and icon strokes produce dozens of intermediate shades, whereas tofu
/// boxes and empty surfaces produce a handful of flat colours.
Future<int> distinctColorCount(WidgetTester tester, Finder finder) {
  return _withCapturedImage(finder, (image) async {
    final pixels = await _rawPixels(image);
    return pixels.toSet().length;
  });
}

Future<Uint32List> _rawPixels(ui.Image image) async {
  final data = await image.toByteData(format: ui.ImageByteFormat.rawRgba);
  if (data == null) {
    throw StateError('golden_shot: could not read the captured pixels.');
  }
  return data.buffer.asUint32List();
}

Future<T> _withCapturedImage<T>(
  Finder finder,
  Future<T> Function(ui.Image image) body,
) async {
  final elements = finder.evaluate();
  if (elements.isEmpty) {
    throw StateError('golden_shot: finder matched no widgets: $finder');
  }
  if (elements.length > 1) {
    throw StateError(
      'golden_shot: finder matched ${elements.length} widgets, expected '
      'exactly one: $finder',
    );
  }

  // captureImage() must be *called* outside runAsync (it walks the live layer
  // tree) and *awaited* inside it — this mirrors what matchesGoldenFile does.
  final imageFuture = captureImage(elements.single);
  final result = await TestWidgetsFlutterBinding.instance.runAsync(() async {
    final image = await imageFuture;
    try {
      return await body(image);
    } finally {
      image.dispose();
    }
  });

  if (result == null) {
    throw StateError('golden_shot: image capture did not complete.');
  }
  return result;
}

/// Clears the duplicate-shot registry. Only useful in this helper's own tests.
@visibleForTesting
void resetShotDigests() => _shotDigests.clear();

/// FNV-1a over the encoded bytes. Not cryptographic — just a cheap identity
/// check for "did these two captures produce the same image".
int _digest(Uint8List bytes) {
  var hash = 0xcbf29ce484222325;
  for (final byte in bytes) {
    hash = (hash ^ byte) * 0x100000001b3;
    hash &= 0xFFFFFFFFFFFFFFFF;
  }
  return hash;
}

// ---------------------------------------------------------------------------
// Path resolution
//
// Everything below resolves paths from the environment rather than hardcoding
// them, so the helper works on macOS, Linux and Windows, inside git worktrees,
// and against whatever icon-package version the lockfile happens to pin.
// ---------------------------------------------------------------------------

String? _cachedMobileRoot;
String? _cachedIconRoot;
String? _cachedFlutterRoot;

/// Absolute path to the `mobile/` package root, with `/` separators.
String mobilePackageRoot() {
  final cached = _cachedMobileRoot;
  if (cached != null) {
    return cached;
  }

  // Primary: ask the package resolver where `package:buzz` lives. Independent
  // of whatever working directory the test runner happened to start in.
  final resolved = _packageRootFromConfig('buzz');
  if (resolved != null && Directory('$resolved/assets/fonts').existsSync()) {
    return _cachedMobileRoot = resolved;
  }

  // Fallback: walk up from the working directory looking for the app's fonts.
  var dir = Directory.current.absolute;
  for (var i = 0; i < 8; i++) {
    final candidate = _stripTrailingSlash(_posix(dir.path));
    if (Directory('$candidate/assets/fonts').existsSync() &&
        File('$candidate/pubspec.yaml').existsSync()) {
      return _cachedMobileRoot = candidate;
    }
    final parent = dir.parent;
    if (parent.path == dir.path) {
      break;
    }
    dir = parent;
  }

  throw StateError(
    'golden_shot: could not locate the mobile package root (the directory '
    'holding pubspec.yaml and assets/fonts). Run the tests from mobile/ with '
    '`flutter test`.',
  );
}

/// Absolute path to the resolved `lucide_icons_flutter` package.
String iconPackageRoot() {
  final cached = _cachedIconRoot;
  if (cached != null) {
    return cached;
  }

  final resolved = _packageRootFromConfig(_kIconPackage);
  if (resolved != null && File('$resolved/assets/lucide.ttf').existsSync()) {
    return _cachedIconRoot = resolved;
  }

  // Fallback: glob the pub cache. The version is never hardcoded — take the
  // highest-sorting `lucide_icons_flutter-*` directory that is present.
  final cacheDir = Directory('${_pubCacheRoot()}/hosted/pub.dev');
  if (cacheDir.existsSync()) {
    final matches =
        cacheDir
            .listSync()
            .whereType<Directory>()
            .map((entry) => _stripTrailingSlash(_posix(entry.path)))
            .where((path) => path.split('/').last.startsWith('$_kIconPackage-'))
            .toList()
          ..sort();
    if (matches.isNotEmpty) {
      return _cachedIconRoot = matches.last;
    }
  }

  throw StateError(
    'golden_shot: could not locate the $_kIconPackage package. Run '
    '`flutter pub get` in mobile/, or point PUB_CACHE at your pub cache.',
  );
}

/// Absolute path to the Flutter SDK checkout that owns the bundled fonts.
String flutterSdkRoot() {
  final cached = _cachedFlutterRoot;
  if (cached != null) {
    return cached;
  }

  bool hasFonts(String path) =>
      Directory('$path/bin/cache/artifacts/material_fonts').existsSync();

  final fromEnv = Platform.environment['FLUTTER_ROOT'];
  if (fromEnv != null && fromEnv.isNotEmpty) {
    final normalized = _stripTrailingSlash(_posix(fromEnv));
    if (hasFonts(normalized)) {
      return _cachedFlutterRoot = normalized;
    }
  }

  // `package:flutter` resolves to <sdk>/packages/flutter.
  final flutterPackage = _packageRootFromConfig('flutter');
  if (flutterPackage != null) {
    final sdk = _parentOf(_parentOf(flutterPackage));
    if (hasFonts(sdk)) {
      return _cachedFlutterRoot = sdk;
    }
  }

  // Last resort: the test runner binary lives under <sdk>/bin/cache/...
  var dir = File(Platform.resolvedExecutable).parent;
  for (var i = 0; i < 10; i++) {
    final candidate = _stripTrailingSlash(_posix(dir.path));
    if (hasFonts(candidate)) {
      return _cachedFlutterRoot = candidate;
    }
    final parent = dir.parent;
    if (parent.path == dir.path) {
      break;
    }
    dir = parent;
  }

  throw StateError(
    'golden_shot: could not locate the Flutter SDK. Set FLUTTER_ROOT to your '
    'Flutter checkout and retry.',
  );
}

/// Reads `.dart_tool/package_config.json` and returns [package]'s root dir.
///
/// Uses [Isolate.packageConfigSync] first so it works regardless of the
/// working directory, then falls back to walking up from the CWD.
String? _packageRootFromConfig(String package) {
  for (final configUri in _packageConfigCandidates()) {
    final file = File.fromUri(configUri);
    if (!file.existsSync()) {
      continue;
    }
    final String contents;
    try {
      contents = file.readAsStringSync();
    } on FileSystemException {
      continue;
    }
    final rootUri = _rootUriFor(contents, package);
    if (rootUri == null) {
      continue;
    }
    final resolved = configUri.resolve(rootUri);
    if (!resolved.isScheme('file')) {
      continue;
    }
    return _stripTrailingSlash(_posix(File.fromUri(resolved).path));
  }
  return null;
}

Iterable<Uri> _packageConfigCandidates() sync* {
  final fromIsolate = _isolatePackageConfig();
  if (fromIsolate != null) {
    yield fromIsolate;
  }
  var dir = Directory.current.absolute;
  for (var i = 0; i < 8; i++) {
    yield Uri.file(
      '${_stripTrailingSlash(_posix(dir.path))}/.dart_tool/package_config.json',
    );
    final parent = dir.parent;
    if (parent.path == dir.path) {
      break;
    }
    dir = parent;
  }
}

/// The package config the isolate was launched with, when it will tell us.
///
/// `flutter_tester` throws `Unsupported operation: Isolate.packageConfig`, so
/// this is strictly a best-effort shortcut — callers fall back to walking up
/// from the working directory.
Uri? _isolatePackageConfig() {
  try {
    final uri = Isolate.packageConfigSync;
    return uri != null && uri.isScheme('file') ? uri : null;
  } on Object {
    return null;
  }
}

/// Pulls `rootUri` for [package] out of a package_config.json body.
///
/// Deliberately regex-based rather than `dart:convert` plus a pile of casts:
/// the file is machine-generated and flat, and this keeps a malformed entry
/// from throwing where a clean "not found" is the useful answer.
String? _rootUriFor(String json, String package) {
  final entry = RegExp(
    '\\{[^{}]*"name"\\s*:\\s*"${RegExp.escape(package)}"[^{}]*\\}',
  ).firstMatch(json);
  if (entry == null) {
    return null;
  }
  final rootUri = RegExp(
    r'"rootUri"\s*:\s*"([^"]*)"',
  ).firstMatch(entry.group(0)!);
  return rootUri?.group(1);
}

String _pubCacheRoot() {
  final fromEnv = Platform.environment['PUB_CACHE'];
  if (fromEnv != null && fromEnv.isNotEmpty) {
    return _stripTrailingSlash(_posix(fromEnv));
  }
  if (Platform.isWindows) {
    final localAppData = Platform.environment['LOCALAPPDATA'];
    if (localAppData != null && localAppData.isNotEmpty) {
      return '${_stripTrailingSlash(_posix(localAppData))}/Pub/Cache';
    }
  }
  final home =
      Platform.environment['HOME'] ?? Platform.environment['USERPROFILE'] ?? '';
  return '${_stripTrailingSlash(_posix(home))}/.pub-cache';
}

String _parentOf(String path) {
  final index = path.lastIndexOf('/');
  return index <= 0 ? path : path.substring(0, index);
}

String _stripTrailingSlash(String path) => path.length > 1 && path.endsWith('/')
    ? path.substring(0, path.length - 1)
    : path;

/// Normalises Windows backslashes so the rest of the file can join with '/'.
/// `dart:io` accepts forward slashes on every platform Flutter supports.
String _posix(String path) => path.replaceAll(r'\', '/');

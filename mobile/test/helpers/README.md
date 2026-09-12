# Mobile test helpers

Shared scaffolding for the Flutter widget tests.

| File | What it is |
|------|------------|
| `widget_helpers.dart` | `WidgetHelpers.testable()` — wraps a widget in `ProviderScope` + `MaterialApp` + `Scaffold` with the app theme. |
| `golden_shot.dart` | Golden-screenshot helper: render any widget at a phone surface with the real app fonts and write a PNG. |

---

## Golden screenshots (`golden_shot.dart`)

PR screenshots for the mobile app normally mean a Mac, Xcode, and a booted
simulator. `golden_shot.dart` removes all three: a widget test rendered at a
phone surface with the real fonts loaded is good enough for review, and
`flutter test` runs everywhere — including Windows.

### Writing a shot

```dart
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/golden_shot.dart';
import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('channel list', (tester) async {
    await loadAppFonts();
    setPhoneSurface(tester);

    await tester.pumpWidget(WidgetHelpers.testable(child: const ChannelList()));

    await captureShot(tester, find.byType(ChannelList), '01-channel-list');
  });
}
```

### Producing the PNGs

```sh
cd mobile
flutter test --update-goldens test/features/channels
```

The PNGs are written to a `goldens/` directory next to the test file, named
after the second argument to `captureShot`. Prefix names with `01-`, `02-`, …
so they sort into the order you want them read.

A plain `flutter test` (no flag) compares against whatever was written, so once
a shot exists it doubles as a visual regression test.

To post the results on a PR, use the repo-root helper — **never** `buzz upload`
or a relay media URL, which fail through GitHub's camo proxy:

```sh
./scripts/post-screenshots.sh <PR-number> mobile/test/features/channels/goldens
```

### Two shots that are byte-identical

`captureShot` throws when two shots in the same run produce identical bytes.
That is nearly always a real bug rather than a coincidence: both captures
rendered the *same* state, because the test never actually drove the UI into
the second state, or because the finder was scoped to a whole page that looks
the same in both. It is the most common screenshot mistake there is, and it is
almost impossible to spot by eye in a wall of thumbnails.

Fix the test. Only pass `failOnDuplicate: false` when two states really are
meant to be pixel-identical.

Outside this helper, the same check by hand is:

```sh
shasum -a 256 mobile/test/**/goldens/*.png   # every hash should be unique
```

---

## The two font traps

`flutter_test` renders with a stub font that draws every glyph as a filled
square. Screenshots taken without registering real fonts are walls of tofu
boxes, and nothing warns you — the test passes, the PNG is just wrong. This is
what `loadAppFonts()` exists to prevent, and there are two ways to get it wrong.

**1. The app font.** `Inter` lives at `mobile/assets/fonts/InterVariable.ttf`
and has to be handed to `FontLoader` explicitly; `pubspec.yaml`'s `fonts:`
section only covers the real app, not the test host.

**2. `fontPackage` (the subtle one).** Icons come from the
`lucide_icons_flutter` package, and every one of its `IconData` constants
carries `fontPackage: 'lucide_icons_flutter'`. Flutter turns that into the
family name:

```
packages/lucide_icons_flutter/Lucide
```

Registering the bare family `Lucide` *succeeds*, logs nothing, and leaves
**every icon in the app** as a tofu box. `loadAppFonts()` registers the
package-qualified names, including the `Lucide100`…`Lucide600` weight variants
behind `LucideIcons.<name><weight>`.

`golden_shot_test.dart` guards both traps by rendering pairs that are identical
under the stub font and obviously different under the real ones (`MMMM` vs
`llll`; two different Lucide icons).

Pairwise inequality is enough for the *base* families — with no font registered
both glyphs collapse to the same box, so the two captures match. It is **not**
enough for the `Lucide100`…`Lucide600` variants: an unregistered `Lucide100`
draws a box, and a box differs from a real bell just as much as a thin bell
does, so the comparison stays green while the icon is broken. Those are checked
by shade count instead — a solid tofu box is 2 flat colours, an antialiased
glyph is ~17. `registeredFontFamilies` is likewise recorded as fonts actually
register, not restated as a literal list, so it cannot report a family that a
later edit stopped registering.

### Two more traps, for whoever edits the helper next

**Font bytes are read synchronously.** `FontLoader.load()` awaits the futures it
was given, and a future that completed with an *error* wedges the run forever
instead of failing it — a missing font file turns into a hung test suite with
no output. `_readFontSync` checks `existsSync()` and throws a message naming
the missing path before any future is created. Keep it that way.

**The golden comparison runs inside `runAsync`.** `captureShot` hands raw bytes
to `matchesGoldenFile`, and on that path flutter_test awaits the golden
comparator *outside* `runAsync` (the early `List<int>` branch of
`MatchesGoldenFile.matchAsync`). `LocalFileComparator` reads and writes with
real `dart:io` futures, and a real I/O future never completes inside the
fake-async zone a widget test runs in. Unwrap that call and
`flutter test --update-goldens` hangs forever with no output rather than
writing a PNG.

Nothing cheap catches this: an in-memory comparator completes synchronously and
looks perfectly healthy, which is why the self-test also drives a comparator
that does real file I/O (`_RealIoGoldens`) under an explicit `Timeout`.
flutter_test disables per-test timeouts by default, so without that timeout a
regression hangs CI instead of failing it.

### Paths

Nothing is hardcoded. The mobile package, the icon package (whatever version
the lockfile pins), and the Flutter SDK are all resolved from
`.dart_tool/package_config.json`, `FLUTTER_ROOT`/`PUB_CACHE`, or the running
executable, so the helper works on macOS, Linux, Windows, and inside git
worktrees.

---

## API

| Function | Purpose |
|----------|---------|
| `loadAppFonts()` | Register Inter, GeistMono, the Lucide icon families, MaterialIcons and Roboto. Idempotent; call it per test. |
| `setPhoneSurface(tester, {logical, dpr})` | Size the view to 390×844 @2x, pin text scale to 1.0, zero the safe-area padding. Restores everything via `addTearDown`. |
| `captureShot(tester, finder, name)` | Settle, rasterise, reject byte-identical duplicates, compare against `goldens/<name>.png`. |
| `rasterize(tester, finder)` | Encoded PNG bytes for a finder, with no golden file involved. |
| `isVisuallyBlank(tester, finder)` | `true` when every pixel is the same colour — the signature of a render that produced nothing. |
| `distinctColorCount(tester, finder)` | Number of distinct colours; a cheap "did the glyphs rasterise" probe. |

### Why there are no committed goldens for the helper itself

`golden_shot_test.dart` is assertion-based, not golden-based. Binary PNGs in
the repo would need refreshing on every font or engine bump and could not
explain *why* a shot broke; the pairwise assertions can. The self-test
therefore passes under a plain `flutter test` with no `--update-goldens` step
and no PNGs checked in.

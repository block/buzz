import 'package:buzz/shared/relay/identity_scoped_prefs.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  const canonical = 'k_v1:https://relay.example:pk';
  const legacy = 'k_v1:wss://relay.example:pk';

  Future<SharedPreferences> prefsWith(Map<String, Object> values) async {
    SharedPreferences.setMockInitialValues(values);
    return SharedPreferences.getInstance();
  }

  String? readString(SharedPreferences prefs) => readMigratedPref<String>(
    prefs,
    canonicalKey: canonical,
    legacyKey: legacy,
    read: prefs.getString,
    write: prefs.setString,
  );

  test('returns the canonical value when present', () async {
    final prefs = await prefsWith({canonical: 'new', legacy: 'old'});
    expect(readString(prefs), 'new');
  });

  test('falls back to the legacy value and returns it immediately', () async {
    final prefs = await prefsWith({legacy: 'carried over'});
    expect(readString(prefs), 'carried over');
  });

  test('promotes the legacy value onto the canonical key', () async {
    final prefs = await prefsWith({legacy: 'carried over'});
    readString(prefs);
    await Future<void>.delayed(Duration.zero);

    expect(prefs.getString(canonical), 'carried over');
    expect(prefs.getString(legacy), isNull, reason: 'legacy entry is cleared');
  });

  test('is idempotent across repeated reads', () async {
    final prefs = await prefsWith({legacy: 'carried over'});
    readString(prefs);
    await Future<void>.delayed(Duration.zero);
    expect(readString(prefs), 'carried over');
    await Future<void>.delayed(Duration.zero);
    expect(prefs.getString(canonical), 'carried over');
  });

  test('returns null when neither key holds a value', () async {
    final prefs = await prefsWith({});
    expect(readString(prefs), isNull);
  });

  test('does not touch storage when the keys coincide', () async {
    // A pairing-created community already stores an HTTP origin, so canonical
    // and legacy are the same string and there is nothing to migrate.
    SharedPreferences.setMockInitialValues({canonical: 'only'});
    final prefs = await SharedPreferences.getInstance();
    final value = readMigratedPref<String>(
      prefs,
      canonicalKey: canonical,
      legacyKey: canonical,
      read: prefs.getString,
      write: prefs.setString,
    );
    await Future<void>.delayed(Duration.zero);

    expect(value, 'only');
    expect(prefs.getString(canonical), 'only');
  });

  test('migrates string lists as well as strings', () async {
    final prefs = await prefsWith({
      legacy: <String>['alpha', 'beta'],
    });
    final value = readMigratedPref<List<String>>(
      prefs,
      canonicalKey: canonical,
      legacyKey: legacy,
      read: prefs.getStringList,
      write: prefs.setStringList,
    );
    await Future<void>.delayed(Duration.zero);

    expect(value, ['alpha', 'beta']);
    expect(prefs.getStringList(canonical), ['alpha', 'beta']);
    expect(prefs.getStringList(legacy), isNull);
  });
}

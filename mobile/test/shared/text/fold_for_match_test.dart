import 'package:buzz/shared/text/fold_for_match.dart';
import 'package:flutter_test/flutter_test.dart';

// Expected values match desktop's foldForMatch
// (desktop/src/shared/lib/foldForMatch.test.mjs), so both clients fold a name
// the same way.
void main() {
  test('folds case and accents like desktop', () {
    for (final (input, expected) in [
      ('Élodie', 'elodie'),
      ('elodie', 'elodie'),
      ('RÉUNION', 'reunion'),
      ('François Ç', 'francois c'),
      ('Réunion', 'reunion'),
      ('Nguyễn Thị', 'nguyen thi'),
      ('Øresund', 'oresund'),
      ('Đorđe', 'dorde'),
      ('Kırmızı', 'kirmizi'),
      ('ŒUVRE Cœur', 'oeuvre coeur'),
      ('Æsir', 'aesir'),
      ('Straße', 'strasse'),
      ('GROẞ', 'gross'),
      ('Ἀθῆναι', 'αθηναι'),
      ('Йошкар-Ола', 'иошкар-ола'),
      ('agent-42 🐝', 'agent-42 🐝'),
      ('日本語', '日本語'),
      ('', ''),
    ]) {
      expect(foldForMatch(input), expected, reason: input);
    }
  });
}

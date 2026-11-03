/// Accent- and case-insensitive text folding for name matching, so `elodie`
/// finds `Élodie` and `reunion` finds `Réunion`.
///
/// Mirrors desktop's `foldForMatch` (`desktop/src/shared/lib/foldForMatch.ts`):
/// each code point is lower-cased, combining marks are removed, and letters
/// are reduced to their base letter, including `ø`→`o`, `đ`→`d`, `ı`→`i`,
/// `œ`→`oe`, `æ`→`ae` and `ß`→`ss`.
///
/// Dart core has no Unicode normalization, so precomposed letters are folded
/// through [_foldGroups] instead of NFD. The table covers every letter that
/// desktop's NFD fold changes in Latin-1 Supplement, Latin Extended-A and -B,
/// Greek and Coptic, Cyrillic, Latin Extended Additional and Greek Extended
/// (U+00C0–U+024F, U+0370–U+04FF, U+1E00–U+1FFF). Letters outside those blocks
/// are only lower-cased, and decomposed input (a letter followed by combining
/// marks) folds the same as on desktop.
library;

/// Base text → the lower-case letters that fold to it.
const _foldGroups = <String, String>{
  'a': 'àáâãäåāăąǎǟǡǻȁȃȧḁạảấầẩẫậắằẳẵặ',
  'ae': 'æǣǽ',
  'b': 'ḃḅḇ',
  'c': 'çćĉċčḉ',
  'd': 'ďđḋḍḏḑḓ',
  'e': 'èéêëēĕėęěȅȇȩḕḗḙḛḝẹẻẽếềểễệ',
  'f': 'ḟ',
  'g': 'ĝğġģǧǵḡ',
  'h': 'ĥȟḣḥḧḩḫẖ',
  'i': 'ìíîïĩīĭįıǐȉȋḭḯỉị',
  'j': 'ĵǰ',
  'k': 'ķǩḱḳḵ',
  'l': 'ĺļľḷḹḻḽ',
  'm': 'ḿṁṃ',
  'n': 'ñńņňǹṅṇṉṋ',
  'o': 'òóôõöøōŏőơǒǫǭǿȍȏȫȭȯȱṍṏṑṓọỏốồổỗộớờởỡợ',
  'oe': 'œ',
  'p': 'ṕṗ',
  'r': 'ŕŗřȑȓṙṛṝṟ',
  's': 'śŝşšșṡṣṥṧṩ',
  'ss': 'ß',
  't': 'ţťțṫṭṯṱẗ',
  'u': 'ùúûüũūŭůűųưǔǖǘǚǜȕȗṳṵṷṹṻụủứừửữự',
  'v': 'ṽṿ',
  'w': 'ŵẁẃẅẇẉẘ',
  'x': 'ẋẍ',
  'y': 'ýÿŷȳẏẙỳỵỷỹ',
  'z': 'źżžẑẓẕ',
  'ſ': 'ẛ',
  'ʒ': 'ǯ',
  'ʹ': 'ʹ',
  'α': 'άἀἁἂἃἄἅἆἇὰάᾀᾁᾂᾃᾄᾅᾆᾇᾰᾱᾲᾳᾴᾶᾷ',
  'ε': 'έἐἑἒἓἔἕὲέ',
  'η': 'ήἠἡἢἣἤἥἦἧὴήᾐᾑᾒᾓᾔᾕᾖᾗῂῃῄῆῇ',
  'ι': 'ΐίϊἰἱἲἳἴἵἶἷὶίιῐῑῒΐῖῗ',
  'ο': 'όὀὁὂὃὄὅὸό',
  'ρ': 'ῤῥ',
  'υ': 'ΰϋύὐὑὒὓὔὕὖὗὺύῠῡῢΰῦῧ',
  'ω': 'ώὠὡὢὣὤὥὦὧὼώᾠᾡᾢᾣᾤᾥᾦᾧῲῳῴῶῷ',
  'ϒ': 'ϓϔ',
  'а': 'ӑӓ',
  'г': 'ѓ',
  'е': 'ѐёӗ',
  'ж': 'ӂӝ',
  'з': 'ӟ',
  'и': 'йѝӣӥ',
  'к': 'ќ',
  'о': 'ӧ',
  'у': 'ўӯӱӳ',
  'ч': 'ӵ',
  'ы': 'ӹ',
  'э': 'ӭ',
  'і': 'ї',
  'ѵ': 'ѷ',
  'ә': 'ӛ',
  'ө': 'ӫ',
};

final _foldTable = <int, String>{
  for (final MapEntry(key: base, value: letters) in _foldGroups.entries)
    for (final rune in letters.runes) rune: base,
};

final _combiningMark = RegExp(r'\p{M}', unicode: true);

/// Folds [s] for matching: lower-cased, accents removed. Use it on both sides
/// of a comparison only; never display, insert or key identities on the
/// result.
String foldForMatch(String s) {
  final buffer = StringBuffer();
  for (final rune in s.runes) {
    for (final lower in String.fromCharCode(rune).toLowerCase().runes) {
      final folded = _foldTable[lower];
      if (folded != null) {
        buffer.write(folded);
        continue;
      }
      final character = String.fromCharCode(lower);
      if (!_combiningMark.hasMatch(character)) buffer.write(character);
    }
  }
  return buffer.toString();
}

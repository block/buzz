import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/verified_profiles/verified_profile.dart';
import 'package:buzz/shared/verified_profiles/verified_profile_proof.dart';
import 'package:flutter_test/flutter_test.dart';

import 'verified_profile_fixture.dart';

VerifiedProfile? parse(NostrEvent event) =>
    parseVerifiedProfile(event, authority, checked: true);

NostrEvent withTags(List<List<String>> tags, {int createdAt = 2000}) =>
    signedProjection(tags, createdAt: createdAt);

void main() {
  group('discovery', () {
    Map<String, Object?> info({Object? fi, bool withFi = true, Object? self}) =>
        {
          'self': self ?? relayKeys.public,
          'federated_identity': ?(withFi ? fi : null),
        };
    Map<String, Object?> pp([Object? kind = 30382, Object? multi = false]) => {
      'public_profile': {'kind': kind, 'multiple_issuers': multi},
    };

    test('enabled, disabled and malformed are distinct', () {
      final enabled = parseVerifiedProfileDiscovery(info(fi: pp(30382, true)));
      expect(enabled, isA<VerifiedProfilesEnabled>());
      expect(
        (enabled as VerifiedProfilesEnabled).authority,
        VerifiedProfileAuthority(
          pubkey: relayKeys.public,
          multipleIssuers: true,
        ),
      );
      expect(
        parseVerifiedProfileDiscovery(info(withFi: false)),
        isA<VerifiedProfilesDisabled>(),
      );
      expect(
        parseVerifiedProfileDiscovery(info(fi: <String, Object?>{})),
        isA<VerifiedProfilesDisabled>(),
      );
      for (final bad in <Object?>[
        'nope',
        info(fi: 'x'),
        info(fi: {'public_profile': 'x'}),
        info(fi: pp(1)),
        info(fi: pp(30382, 'yes')),
        info(fi: pp(), self: 'A' * 64),
        info(fi: pp(), self: 'abc'),
      ]) {
        expect(
          () => parseVerifiedProfileDiscovery(bad),
          throwsFormatException,
          reason: '$bad',
        );
      }
    });
  });

  group('names', () {
    test('accepts and rejects per NIP-FI', () {
      for (final ok in ['Ada', 'Zoë Smith', '日本語', 'a' * 64]) {
        expect(isValidVerifiedName(ok), isTrue, reason: ok);
      }
      for (final bad in [
        '',
        'a' * 65,
        ' Ada',
        'Ada ',
        'Ada ',
        'A\u0000da',
        'A​da',
        'Zoë',
        'A da',
        'Ada',
      ]) {
        expect(
          isValidVerifiedName(bad),
          isFalse,
          reason: bad.codeUnits.join(','),
        );
      }
    });
  });

  group('projection', () {
    test('a well-formed projection parses, with and without full_name', () {
      final profile = parse(withTags(profileTags(subjectA)))!;
      expect(profile.displayName, 'Ada Lovelace');
      expect(profile.username, 'ada');
      expect(profile.issuerLabel, 'Example Corp');
      expect(profile.verifiedAt, 1000);
      expect(profile.fullName, isNull);
      final full = parse(withTags(profileTags(subjectA, fullName: 'Ada K')))!;
      expect(full.fullName, 'Ada K');
    });

    test('rejects every deviation from the event shape', () {
      final good = profileTags(subjectA);
      List<List<String>> without(String name) => [
        for (final t in good)
          if (t.first != name) t,
      ];
      List<List<String>> replace(String name, String value) => [
        for (final t in good) t.first == name ? [name, value] : t,
      ];
      final cases = <String, NostrEvent>{
        'wrong author': signedProjection(good, author: relayKeysOther()),
        'retraction': withTags(retractionTags(subjectA)),
        'p differs from d': withTags(replace('p', subjectB)),
        'duplicate tag': withTags([
          ...good,
          ['username', 'ada2'],
        ]),
        'extra tag': withTags([
          ...good,
          ['x', 'y'],
        ]),
        'two full_name': withTags([
          ...good,
          ['full_name', 'A'],
          ['full_name', 'B'],
        ]),
        'three-element tag': withTags([
          for (final t in good) t.first == 'p' ? [...t, 'x'] : t,
        ]),
        'missing display_name': withTags([
          ...without('display_name'),
          ['x', 'y'],
        ]),
        'bad username': withTags(replace('username', 'Ada!')),
        'long username': withTags(replace('username', 'a' * 65)),
        'non-NFC name': withTags(replace('display_name', 'Zoë')),
        'padded name': withTags(replace('display_name', ' Ada')),
        'bad issuer': withTags(replace('issuer_label', '')),
        'bad full_name': withTags([
          ...good,
          ['full_name', ' x'],
        ]),
        'inactive text': withTags(replace('active', 'TRUE')),
        'verified_at after created_at': withTags(
          replace('verified_at', '2001'),
        ),
        'verified_at not decimal': withTags(replace('verified_at', '1e3')),
        'verified_at negative': withTags(replace('verified_at', '-5')),
        'uppercase subject': withTags([
          ['d', 'A' * 64],
          ...good.skip(1),
        ]),
        'content not empty': signedProjection(good, content: 'hi'),
      };
      for (final MapEntry(:key, :value) in cases.entries) {
        expect(parse(value), isNull, reason: key);
      }
    });

    test('verified_at equal to created_at is accepted', () {
      expect(
        parse(withTags(profileTags(subjectA, verifiedAt: '2000'))),
        isNotNull,
      );
    });
  });

  group('ordering and proof', () {
    test('newer created_at wins; a tie goes to the lower id', () {
      final older = withTags(profileTags(subjectA), createdAt: 10);
      final newer = withTags(profileTags(subjectA, name: 'New'), createdAt: 11);
      expect(supersedes(newer, older), isTrue);
      expect(supersedes(older, newer), isFalse);
      final x = withTags(profileTags(subjectA, name: 'X'), createdAt: 20);
      final y = withTags(profileTags(subjectA, name: 'Y'), createdAt: 20);
      final lower = x.id.compareTo(y.id) < 0 ? x : y;
      final higher = identical(lower, x) ? y : x;
      expect(supersedes(lower, higher), isTrue);
      expect(supersedes(higher, lower), isFalse);
      expect(supersedes(older, null), isTrue);
    });

    test('signature and id are verified', () {
      final event = withTags(profileTags(subjectA));
      expect(hasVerifiedEventProof(event), isTrue);
      final forgedSig = NostrEvent(
        id: event.id,
        pubkey: event.pubkey,
        createdAt: event.createdAt,
        kind: event.kind,
        tags: event.tags,
        content: event.content,
        sig: '0' * 128,
      );
      expect(hasVerifiedEventProof(forgedSig), isFalse);
      final tampered = NostrEvent(
        id: event.id,
        pubkey: event.pubkey,
        createdAt: event.createdAt,
        kind: event.kind,
        tags: profileTags(subjectA, name: 'Mallory'),
        content: event.content,
        sig: event.sig,
      );
      expect(hasVerifiedEventProof(tampered), isFalse);
    });
  });
}

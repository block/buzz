import 'dart:typed_data';

import 'package:buzz/shared/relay/transcription.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;

Map<String, Object?> _info({
  Object? extensions = const ['buzz-transcription'],
  Object? path = '/audio/transcriptions',
}) => {
  'supported_extensions': extensions,
  'transcription': {'path': path},
};

void main() {
  test('reads only a usable, relay-relative transcription capability', () {
    final usable = TranscriptionCapability.fromRelayInformation(_info());
    expect(usable?.path, '/audio/transcriptions');

    // Each row must hide dictation rather than send audio somewhere else.
    final rejected = <String, Object?>{
      'not a document': 'nope',
      'extension missing': _info(extensions: const ['nip-11']),
      'no descriptor': {
        'supported_extensions': const ['buzz-transcription'],
      },
      'absolute URL': _info(path: 'https://elsewhere.example/stt'),
      'protocol-relative': _info(path: '//elsewhere.example/stt'),
      'non-string path': _info(path: 7),
    };
    for (final MapEntry(key: name, value: info) in rejected.entries) {
      expect(
        TranscriptionCapability.fromRelayInformation(info),
        isNull,
        reason: name,
      );
    }
  });

  test('fails what the composer journey never sends or sees', () async {
    final keys = nostr.Keys.generate();
    final clip = Uint8List.fromList([1, 2, 3]);

    // The composer journey covers transcripts, drops, a non-2xx status, 413
    // and 429; these only this client reaches.
    final cases =
        <String, (Uint8List, Future<http.Response> Function(), bool, Object?)>{
          // Rejected locally: an upload would have succeeded.
          'over the relay limit': (
            Uint8List(maxTranscriptionAudioBytes + 1),
            () async => http.Response('{"text":"uploaded"}', 200),
            true,
            TranscriptionFailure.tooLarge,
          ),
          'not JSON': (
            clip,
            () async => http.Response('nope', 200),
            true,
            TranscriptionFailure.service,
          ),
          'no text': (
            clip,
            () async => http.Response('{"x":1}', 200),
            true,
            TranscriptionFailure.service,
          ),
          'no signing key': (
            clip,
            () async => http.Response('{"text":"signed"}', 200),
            false,
            TranscriptionFailure.service,
          ),
        };
    for (final MapEntry(key: name, value: (audio, answer, signedIn, expected))
        in cases.entries) {
      final client = TranscriptionClient(
        clientFactory: () => http_testing.MockClient((_) => answer()),
        baseUrl: 'http://localhost:3000',
        nsec: signedIn ? keys.nsec : null,
        pubkey: signedIn ? keys.public : null,
      );
      final failure = await client
          .transcribe(
            audio,
            capability: const TranscriptionCapability(
              path: '/audio/transcriptions',
            ),
          )
          .then<TranscriptionFailure?>(
            (_) => null,
            onError: (Object error) =>
                (error as TranscriptionException).failure,
          );
      expect(failure, expected, reason: name);
    }
  });
}

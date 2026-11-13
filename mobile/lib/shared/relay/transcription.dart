import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import '../hosted/hosted_relay_auth.dart';
import 'nip11_document.dart';
import 'relay_author_provider.dart';
import 'relay_provider.dart';
import 'relay_session.dart' show buildNip98AuthHeader;

/// NIP-11 extension a relay advertises when it transcribes dictation.
const transcriptionExtension = 'buzz-transcription';

/// Content type of the composer's AAC/M4A recordings.
const transcriptionContentType = 'audio/mp4';

/// How long one transcription request may take, upload included. Longer than
/// the relay's 60-second upstream deadline so its own error arrives first.
const transcriptionTimeout = Duration(seconds: 75);

/// The largest audio the relay accepts (its `MAX_AUDIO_BYTES`). Larger audio
/// fails as [TranscriptionFailure.tooLarge] without being uploaded.
const maxTranscriptionAudioBytes = 5 * 1024 * 1024;

/// A relay's advertised speech-to-text endpoint.
class TranscriptionCapability {
  const TranscriptionCapability({required this.path});

  /// Relay-relative endpoint path.
  final String path;

  /// Reads the capability from a NIP-11 document, or null when the relay
  /// does not advertise one the app can use: the extension is missing or the
  /// path is not a plain relay-relative path.
  static TranscriptionCapability? fromRelayInformation(Object? information) {
    if (information is! Map) return null;
    final extensions = information['supported_extensions'];
    if (extensions is! List || !extensions.contains(transcriptionExtension)) {
      return null;
    }
    final descriptor = information['transcription'];
    if (descriptor is! Map) return null;
    final path = descriptor['path'];
    if (path is! String || !path.startsWith('/') || path.startsWith('//')) {
      return null;
    }
    return TranscriptionCapability(path: path);
  }
}

/// The active community relay's transcription capability, or null when it
/// has none. Dictation stays hidden while a failed read is retried by the
/// root `ProviderScope` policy (`relayProviderRetry`); a successful read is
/// kept until the community changes.
final transcriptionCapabilityProvider =
    FutureProvider<TranscriptionCapability?>((ref) async {
      final baseUrl = ref.watch(
        relayConfigProvider.select((config) => config.baseUrl),
      );
      return TranscriptionCapability.fromRelayInformation(
        await fetchNip11Document(
          ref.watch(relayInformationClientProvider),
          Uri.parse(baseUrl),
        ),
      );
    });

/// Why a transcription request failed, for user-facing messages.
enum TranscriptionFailure {
  /// The request never reached the relay or timed out.
  network,

  /// The relay or its provider refused or failed the request.
  service,

  /// The member hit the relay's transcription rate limit.
  rateLimited,

  /// The recording is over [maxTranscriptionAudioBytes], or the relay
  /// answered 413.
  tooLarge,
}

/// A failed transcription request. Carries no audio or transcript text.
class TranscriptionException implements Exception {
  const TranscriptionException(this.failure);

  final TranscriptionFailure failure;

  @override
  String toString() => 'TranscriptionException(${failure.name})';
}

/// Sends dictated audio to the community relay and returns the transcript.
class TranscriptionClient {
  TranscriptionClient({
    required http.Client Function() clientFactory,
    required String baseUrl,
    required String? nsec,
    required String? pubkey,
  }) : _clientFactory = clientFactory,
       _baseUrl = baseUrl,
       _nsec = nsec,
       _pubkey = pubkey;

  final http.Client Function() _clientFactory;
  final String _baseUrl;
  final String? _nsec;
  final String? _pubkey;

  /// Uploads [audio] to [capability] with NIP-98 auth bound to the body.
  /// Completing [cancelled] aborts the request by closing its client.
  /// Throws [TranscriptionException] on any failure, and before uploading
  /// anything over [maxTranscriptionAudioBytes].
  Future<String> transcribe(
    Uint8List audio, {
    required TranscriptionCapability capability,
    Future<void>? cancelled,
  }) async {
    if (audio.length > maxTranscriptionAudioBytes) {
      throw const TranscriptionException(TranscriptionFailure.tooLarge);
    }
    final url = Uri.parse(_baseUrl).resolve(capability.path);
    final String authorization;
    try {
      authorization = buildNip98AuthHeader(
        method: 'POST',
        url: url.toString(),
        bodyBytes: audio,
        nsec: _nsec,
        pubkey: _pubkey,
      );
    } catch (_) {
      throw const TranscriptionException(TranscriptionFailure.service);
    }
    final client = _clientFactory();
    unawaited(cancelled?.then((_) => client.close()));
    final http.Response response;
    try {
      response = await client
          .post(
            url,
            headers: {
              'Authorization': authorization,
              'Content-Type': transcriptionContentType,
            },
            body: audio,
          )
          .timeout(transcriptionTimeout);
    } catch (_) {
      throw const TranscriptionException(TranscriptionFailure.network);
    } finally {
      client.close();
    }
    if (response.statusCode == 429) {
      throw const TranscriptionException(TranscriptionFailure.rateLimited);
    }
    if (response.statusCode == 413) {
      throw const TranscriptionException(TranscriptionFailure.tooLarge);
    }
    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw const TranscriptionException(TranscriptionFailure.service);
    }
    try {
      final decoded = jsonDecode(utf8.decode(response.bodyBytes));
      final text = decoded is Map ? decoded['text'] : null;
      if (text is String) return text;
    } on FormatException {
      // Falls through to the service failure below.
    }
    throw const TranscriptionException(TranscriptionFailure.service);
  }
}

/// The active community's transcription client. Requests carry the build's
/// hosted relay assertions the same way other relay HTTP calls do.
final transcriptionClientProvider = Provider<TranscriptionClient>((ref) {
  final config = ref.watch(relayConfigProvider);
  return TranscriptionClient(
    clientFactory: ref.watch(hostedHttpClientFactoryProvider),
    baseUrl: config.baseUrl,
    nsec: config.nsec,
    pubkey: ref.watch(myPubkeyProvider),
  );
});

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;

import 'package:file_selector/file_selector.dart' as file_selector;
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:image_picker/image_picker.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';

import 'animated_image_sanitizer.dart';
import 'media_auth.dart';
import 'mp4_fast_start.dart';
import 'relay_provider.dart';

const _mediaUploadPath = '/upload';
const _legacyMediaUploadPath = '/media/upload';
const _mediaUploadPlatformChannelName = 'buzz/media_upload';
const _sanitizeImageForUploadMethod = 'sanitizeImageForUpload';
const _transcodeVideoToMp4Method = 'transcodeVideoToMp4';
const _generateVideoPosterMethod = 'generateVideoPoster';
const _transcodeImageToJpegMethod = 'transcodeImageToJpeg';
const _pickAttachmentFileMethod = 'pickAttachmentFile';
const _requiresLegacyMediaStoragePermissionMethod =
    'requiresLegacyMediaStoragePermission';
const _readClipboardImageMethod = 'readClipboardImage';
const _clipboardHasImageMethod = 'clipboardHasImage';
const _uploadAuthKind = 24242;
const _uploadAuthLifetimeSeconds = 300;
const _lanMediaProbeTimeout = Duration(seconds: 2);
const _heicBrands = {
  'heic',
  'heix',
  'hevc',
  'hevx',
  'heim',
  'heis',
  'mif1',
  'msf1',
};
final _mediaUploadPlatformChannel = MethodChannel(
  _mediaUploadPlatformChannelName,
);

/// Whether saving media needs Android's pre-scoped-storage runtime permission.
Future<bool> requiresLegacyMediaStoragePermission() async {
  if (defaultTargetPlatform != TargetPlatform.android) {
    return false;
  }
  return await _mediaUploadPlatformChannel.invokeMethod<bool>(
        _requiresLegacyMediaStoragePermissionMethod,
      ) ??
      false;
}

const _allowedImageMimeTypes = {
  'image/jpeg',
  'image/png',
  'image/gif',
  'image/webp',
};
const _allowedVideoMimeTypes = {'video/mp4'};
const _maxVideoSizeBytes = 100 * 1024 * 1024; // 100MB
const _maxFileSizeBytes = 100 * 1024 * 1024; // 100MB
const _mediaPolicyUploadMessage = "We couldn't prepare this image for upload.";

typedef PickGalleryImage = Future<XFile?> Function();

/// Selects multiple gallery images for upload in picker order.
typedef PickGalleryImages = Future<List<XFile>> Function();
typedef PickGalleryVideo = Future<XFile?> Function();
typedef PickAttachmentFile = Future<XFile?> Function();
typedef SanitizeImageBytes =
    Future<Uint8List> Function(Uint8List bytes, String mimeType);
typedef TranscodeImageToJpeg = Future<Uint8List> Function(Uint8List bytes);
typedef TranscodeVideoToMp4 = Future<String> Function(String filePath);

/// Generates poster-frame bytes for the video at [filePath], when available.
typedef GenerateVideoPoster = Future<Uint8List?> Function(String filePath);
typedef ReadClipboardImage = Future<Uint8List?> Function();

class MediaPolicyUploadException implements Exception {
  const MediaPolicyUploadException();

  @override
  String toString() => _mediaPolicyUploadMessage;
}

/// Cancels a single user-initiated media upload without closing the shared
/// HTTP client used by later uploads.
class UploadCancellationToken {
  final Completer<void> _cancelled = Completer<void>();

  /// Whether cancellation has been requested.
  bool get isCancelled => _cancelled.isCompleted;

  /// Completes when cancellation is first requested.
  Future<void> get whenCancelled => _cancelled.future;

  /// Requests cancellation. Calling this more than once has no effect.
  void cancel() {
    if (!_cancelled.isCompleted) _cancelled.complete();
  }
}

/// Indicates that a user cancelled a media upload before it completed.
class UploadCancelledException implements Exception {
  const UploadCancelledException();
}

@immutable
class _PreparedUploadImage {
  final Uint8List bytes;
  final String mimeType;

  const _PreparedUploadImage({required this.bytes, required this.mimeType});
}

@immutable
class BlobDescriptor {
  final String url;
  final String sha256;
  final int size;
  final String type;
  final int uploaded;
  final String? dim;
  final String? blurhash;
  final String? thumb;
  final double? duration;
  final String? image;
  final String? filename;

  const BlobDescriptor({
    required this.url,
    required this.sha256,
    required this.size,
    required this.type,
    required this.uploaded,
    this.dim,
    this.blurhash,
    this.thumb,
    this.duration,
    this.image,
    this.filename,
  });

  factory BlobDescriptor.fromJson(Map<String, dynamic> json) => BlobDescriptor(
    url: json['url'] as String,
    sha256: json['sha256'] as String,
    size: (json['size'] as num).toInt(),
    type: json['type'] as String,
    uploaded: (json['uploaded'] as num).toInt(),
    dim: json['dim'] as String?,
    blurhash: json['blurhash'] as String?,
    thumb: json['thumb'] as String?,
    duration: (json['duration'] as num?)?.toDouble(),
    image: json['image'] as String?,
    filename: json['filename'] as String?,
  );

  BlobDescriptor withFilename(String value) => BlobDescriptor(
    url: url,
    sha256: sha256,
    size: size,
    type: type,
    uploaded: uploaded,
    dim: dim,
    blurhash: blurhash,
    thumb: thumb,
    duration: duration,
    image: image,
    filename: value,
  );

  /// Returns a descriptor with [value] as its NIP-71 video poster URL.
  BlobDescriptor withImage(String value) => BlobDescriptor(
    url: url,
    sha256: sha256,
    size: size,
    type: type,
    uploaded: uploaded,
    dim: dim,
    blurhash: blurhash,
    thumb: thumb,
    duration: duration,
    image: value,
    filename: filename,
  );

  List<String> toImetaTag() => [
    'imeta',
    'url $url',
    'm $type',
    'x $sha256',
    'size $size',
    if (dim != null) 'dim $dim',
    if (blurhash != null) 'blurhash $blurhash',
    if (thumb != null) 'thumb $thumb',
    if (duration != null) 'duration $duration',
    if (image != null) 'image $image',
    if (filename != null) 'filename $filename',
  ];

  String toMarkdownImage() {
    if (type.startsWith('video/')) return '![video]($url)';
    if (type.startsWith('image/')) return '![image]($url)';
    final label = (filename ?? 'file').replaceAllMapped(
      RegExp(r'[\\\[\]]'),
      (match) => '\\${match[0]}',
    );
    return '[$label]($url)';
  }
}

class MediaUploadService {
  final String _baseUrl;
  final List<String> _uploadBaseUrls;
  final String? _nsec;
  final PickGalleryImage _pickGalleryImage;
  final PickGalleryImages _pickGalleryImages;
  final PickGalleryVideo _pickGalleryVideo;
  final PickAttachmentFile? _pickAttachmentFile;
  final SanitizeImageBytes _sanitizeImageBytes;
  final TranscodeImageToJpeg _transcodeImageToJpeg;
  final TranscodeVideoToMp4 _transcodeVideoToMp4;
  final GenerateVideoPoster _generateVideoPoster;
  final ReadClipboardImage _readClipboardImage;
  final DateTime Function() _now;
  final http.Client _http;
  final bool _ownsHttpClient;
  final Set<String> _confirmedUploadBaseUrls = {};

  MediaUploadService({
    required String baseUrl,
    List<String>? baseUrls,
    required String? nsec,
    required PickGalleryImage pickGalleryImage,
    PickGalleryImages? pickGalleryImages,
    required PickGalleryVideo pickGalleryVideo,
    PickAttachmentFile? pickAttachmentFile,
    SanitizeImageBytes? sanitizeImageBytes,
    TranscodeImageToJpeg? transcodeImageToJpeg,
    TranscodeVideoToMp4? transcodeVideoToMp4,
    GenerateVideoPoster? generateVideoPoster,
    ReadClipboardImage? readClipboardImage,
    DateTime Function()? now,
    http.Client? httpClient,
  }) : _baseUrl = baseUrl,
       _uploadBaseUrls = _normalizeUploadBaseUrls(baseUrl, baseUrls),
       _nsec = nsec,
       _pickGalleryImage = pickGalleryImage,
       _pickGalleryImages =
           pickGalleryImages ??
           (() async {
             final image = await pickGalleryImage();
             return image == null ? const <XFile>[] : [image];
           }),
       _pickGalleryVideo = pickGalleryVideo,
       _pickAttachmentFile = pickAttachmentFile,
       _sanitizeImageBytes = sanitizeImageBytes ?? _sanitizePickedImageBytes,
       _transcodeImageToJpeg =
           transcodeImageToJpeg ?? _transcodePickedImageToJpeg,
       _transcodeVideoToMp4 = transcodeVideoToMp4 ?? _transcodePickedVideoToMp4,
       _generateVideoPoster = generateVideoPoster ?? _generatePickedVideoPoster,
       _readClipboardImage = readClipboardImage ?? _readPlatformClipboardImage,
       _now = now ?? DateTime.now,
       _http = httpClient ?? http.Client(),
       _ownsHttpClient = httpClient == null;

  void dispose() {
    if (_ownsHttpClient) {
      _http.close();
    }
  }

  Future<BlobDescriptor?> pickAndUploadImage() async {
    final pickedImage = await _pickGalleryImage();
    if (pickedImage == null) return null;
    return uploadImage(pickedImage);
  }

  /// Opens the system picker with multi-selection enabled.
  Future<List<XFile>> pickGalleryImages() => _pickGalleryImages();

  Future<BlobDescriptor> uploadImage(
    XFile image, {
    ValueChanged<double>? onProgress,
    UploadCancellationToken? cancellationToken,
  }) async {
    final preparedImage = await _prepareUploadImage(image);
    _throwIfCancelled(cancellationToken);
    return _uploadPreparedBytes(
      preparedImage.bytes,
      mimeType: preparedImage.mimeType,
      onProgress: onProgress,
      cancellationToken: cancellationToken,
    );
  }

  Future<bool> clipboardHasImage() async {
    return await _mediaUploadPlatformChannel.invokeMethod<bool>(
          _clipboardHasImageMethod,
        ) ??
        false;
  }

  Future<BlobDescriptor> readAndUploadClipboardImage() async {
    final image = await readClipboardImage();
    if (image == null) throw Exception('Unable to read pasted image');
    return uploadImage(image);
  }

  /// Reads a clipboard image for composer preview before the user sends it.
  Future<XFile?> readClipboardImage() async {
    final bytes = await _readClipboardImage();
    if (bytes == null || bytes.isEmpty) return null;
    return XFile.fromData(bytes, name: 'Pasted image');
  }

  /// Opens the system gallery video picker.
  Future<XFile?> pickGalleryVideo() => _pickGalleryVideo();

  /// Sanitizes and uploads [pickedVideo] as an MP4 attachment.
  Future<BlobDescriptor> uploadVideo(
    XFile pickedVideo, {
    ValueChanged<double>? onProgress,
    UploadCancellationToken? cancellationToken,
  }) async {
    _throwIfCancelled(cancellationToken);
    final length = await pickedVideo.length();
    if (length > _maxVideoSizeBytes) {
      throw Exception(
        'Video is too large (${(length / 1024 / 1024).toStringAsFixed(0)}MB). Maximum is 100MB.',
      );
    }

    // Always rebuild the container. Passing an existing MP4 through would retain
    // QuickTime GPS, global metadata, chapters, or non-A/V tracks.
    String? transcodedPath;
    try {
      transcodedPath = await _transcodeVideoToMp4(pickedVideo.path);
      _throwIfCancelled(cancellationToken);
      final transcodedFile = File(transcodedPath);
      final transcodedLength = await transcodedFile.length();
      if (transcodedLength > _maxVideoSizeBytes) {
        throw Exception(
          'Transcoded video is too large (${(transcodedLength / 1024 / 1024).toStringAsFixed(0)}MB). Maximum is 100MB.',
        );
      }
      final bytes = await transcodedFile.readAsBytes();
      _throwIfCancelled(cancellationToken);
      final video = await uploadBytes(
        bytes,
        mimeType: 'video/mp4',
        onProgress: onProgress == null
            ? null
            : (progress) => onProgress(progress * 0.9),
        cancellationToken: cancellationToken,
      );

      // Extract from the canonical output first so the poster matches the
      // uploaded orientation. Some AVFoundation exports need a moment before
      // their first frame is seekable; fall back to the picked source instead
      // of silently sending a permanently gray video card.
      Uint8List? posterBytes;
      Object? posterExtractionError;
      for (final sourcePath in {transcodedPath, pickedVideo.path}) {
        try {
          final candidate = await _generateVideoPoster(sourcePath);
          if (candidate != null && candidate.isNotEmpty) {
            posterBytes = candidate;
            break;
          }
        } catch (error) {
          posterExtractionError = error;
        }
      }

      if (posterBytes == null) {
        if (posterExtractionError != null) {
          debugPrint('Unable to generate video poster: $posterExtractionError');
        }
        onProgress?.call(1);
        return video;
      }

      // Posters are best-effort: a video that passed the media policy should
      // still send if the separate preview upload fails.
      try {
        _throwIfCancelled(cancellationToken);
        final poster = await uploadImage(
          XFile.fromData(
            posterBytes,
            mimeType: 'image/jpeg',
            name: 'video-poster.jpg',
          ),
          onProgress: onProgress == null
              ? null
              : (progress) => onProgress(0.9 + (progress * 0.1)),
          cancellationToken: cancellationToken,
        );
        return video.withImage(poster.url);
      } catch (error) {
        debugPrint('Unable to upload video poster: $error');
        onProgress?.call(1);
        return video;
      }
    } finally {
      if (transcodedPath != null) {
        try {
          await File(transcodedPath).delete();
        } catch (_) {
          // Best-effort temp file cleanup.
        }
      }
    }
  }

  Future<BlobDescriptor?> pickAndUploadVideo() async {
    final pickedVideo = await pickGalleryVideo();
    if (pickedVideo == null) return null;
    return uploadVideo(pickedVideo);
  }

  /// Opens the system document picker for a generic file attachment.
  Future<XFile?> pickAttachmentFile() async {
    final pickAttachmentFile = _pickAttachmentFile;
    if (pickAttachmentFile == null) {
      throw Exception("File attachments aren't available on this device.");
    }
    return pickAttachmentFile();
  }

  /// Uploads [pickedFile] as a size-limited generic attachment.
  Future<BlobDescriptor> uploadFile(
    XFile pickedFile, {
    ValueChanged<double>? onProgress,
    UploadCancellationToken? cancellationToken,
  }) async {
    final totalStopwatch = Stopwatch()..start();
    _throwIfCancelled(cancellationToken);
    final length = await pickedFile.length();
    if (length == 0) {
      throw Exception('File is empty.');
    }
    if (length > _maxFileSizeBytes) {
      throw Exception(
        'File is too large (${(length / 1024 / 1024).toStringAsFixed(0)}MB). Maximum is 100MB.',
      );
    }
    final readStopwatch = Stopwatch()..start();
    final bytes = await pickedFile.readAsBytes();
    _debugMediaUpload(
      'file read bytes=$length elapsedMs=${readStopwatch.elapsedMilliseconds}',
    );
    _throwIfCancelled(cancellationToken);
    final filename = _safeAttachmentFilename(pickedFile.name);
    final descriptor = await _uploadPreparedBytes(
      bytes,
      mimeType: 'application/octet-stream',
      allowGenericFile: true,
      filename: filename,
      onProgress: onProgress,
      cancellationToken: cancellationToken,
    );
    _debugMediaUpload(
      'file complete bytes=$length totalMs=${totalStopwatch.elapsedMilliseconds}',
    );
    return descriptor.withFilename(filename);
  }

  Future<BlobDescriptor?> pickAndUploadFile() async {
    final pickedFile = await pickAttachmentFile();
    if (pickedFile == null) return null;
    return uploadFile(pickedFile);
  }

  Future<BlobDescriptor> uploadBytes(
    Uint8List bytes, {
    required String mimeType,
    ValueChanged<double>? onProgress,
    UploadCancellationToken? cancellationToken,
  }) async {
    _throwIfCancelled(cancellationToken);
    if (mimeType == 'image/gif' ||
        (mimeType == 'image/png' && _isAnimatedPng(bytes)) ||
        (mimeType == 'image/webp' && _isAnimatedWebp(bytes))) {
      try {
        bytes = sanitizeAnimatedImageForUpload(bytes, mimeType);
      } on FormatException {
        throw Exception('failed to sanitize image for upload');
      }
    }
    return _uploadPreparedBytes(
      bytes,
      mimeType: mimeType,
      onProgress: onProgress,
      cancellationToken: cancellationToken,
    );
  }

  Future<BlobDescriptor> _uploadPreparedBytes(
    Uint8List bytes, {
    required String mimeType,
    bool allowGenericFile = false,
    String? filename,
    ValueChanged<double>? onProgress,
    UploadCancellationToken? cancellationToken,
  }) async {
    _throwIfCancelled(cancellationToken);
    if (!allowGenericFile &&
        !_allowedImageMimeTypes.contains(mimeType) &&
        !_allowedVideoMimeTypes.contains(mimeType)) {
      throw Exception('unsupported file type: $mimeType');
    }

    final sha256 = _sha256Hex(bytes);
    Object? lastTransportError;
    _debugMediaUpload(
      'begin type=$mimeType bytes=${bytes.length} targets='
      '${_uploadBaseUrls.map(_uploadTargetLabel).join(',')}',
    );
    for (var index = 0; index < _uploadBaseUrls.length; index++) {
      final requestBaseUrl = _uploadBaseUrls[index];
      final hasFallback = index + 1 < _uploadBaseUrls.length;
      final isLanCandidate = requestBaseUrl != _baseUrl;

      if (isLanCandidate &&
          !_confirmedUploadBaseUrls.contains(requestBaseUrl)) {
        final probeStopwatch = Stopwatch()..start();
        final available = await _probeUploadBaseUrl(requestBaseUrl);
        _debugMediaUpload(
          'probe target=${_uploadTargetLabel(requestBaseUrl)} '
          'available=$available elapsedMs=${probeStopwatch.elapsedMilliseconds}',
        );
        if (!available) {
          lastTransportError = TimeoutException(
            'LAN media transport is unavailable: $requestBaseUrl',
          );
          continue;
        }
      }

      late http.Response response;
      try {
        response = await _sendUploadRequest(
          bytes: bytes,
          mimeType: mimeType,
          sha256: sha256,
          filename: filename,
          path: _mediaUploadPath,
          requestBaseUrl: requestBaseUrl,
          onProgress: onProgress,
          cancellationToken: cancellationToken,
        );
        if (response.statusCode == HttpStatus.notFound ||
            response.statusCode == HttpStatus.methodNotAllowed) {
          response = await _sendUploadRequest(
            bytes: bytes,
            mimeType: mimeType,
            sha256: sha256,
            filename: filename,
            path: _legacyMediaUploadPath,
            requestBaseUrl: requestBaseUrl,
            onProgress: onProgress,
            cancellationToken: cancellationToken,
          );
        }
      } on http.ClientException catch (error) {
        _confirmedUploadBaseUrls.remove(requestBaseUrl);
        _throwIfCancelled(cancellationToken);
        if (!hasFallback) rethrow;
        lastTransportError = error;
        continue;
      } on SocketException catch (error) {
        _confirmedUploadBaseUrls.remove(requestBaseUrl);
        _throwIfCancelled(cancellationToken);
        if (!hasFallback) rethrow;
        lastTransportError = error;
        continue;
      }

      if (response.statusCode >= 200 && response.statusCode < 300) {
        _confirmedUploadBaseUrls.add(requestBaseUrl);
        return BlobDescriptor.fromJson(
          jsonDecode(response.body) as Map<String, dynamic>,
        );
      }
      if (hasFallback &&
          (response.statusCode == HttpStatus.unauthorized ||
              response.statusCode == HttpStatus.notFound ||
              response.statusCode == HttpStatus.methodNotAllowed ||
              response.statusCode >= 500)) {
        lastTransportError = Exception(
          'upload failed (${response.statusCode}): ${response.body}',
        );
        continue;
      }
      if (_allowedImageMimeTypes.contains(mimeType) &&
          (response.statusCode == HttpStatus.unsupportedMediaType ||
              response.statusCode == HttpStatus.unprocessableEntity)) {
        throw const MediaPolicyUploadException();
      }
      throw Exception(
        'upload failed (${response.statusCode}): ${response.body}',
      );
    }

    throw lastTransportError ??
        StateError('No relay media upload transport available');
  }

  Future<bool> _probeUploadBaseUrl(String requestBaseUrl) async {
    try {
      final response = await _http
          .get(
            Uri.parse(requestBaseUrl).resolve('/'),
            headers: {HttpHeaders.hostHeader: Uri.parse(_baseUrl).authority},
          )
          .timeout(_lanMediaProbeTimeout);
      return response.statusCode >= 200 &&
          response.statusCode < 500 &&
          response.statusCode != HttpStatus.notFound;
    } on TimeoutException {
      return false;
    } on http.ClientException {
      return false;
    } on SocketException {
      return false;
    }
  }

  Future<http.Response> _sendUploadRequest({
    required Uint8List bytes,
    required String mimeType,
    required String sha256,
    required String path,
    required String requestBaseUrl,
    String? filename,
    ValueChanged<double>? onProgress,
    UploadCancellationToken? cancellationToken,
  }) async {
    final stopwatch = Stopwatch()..start();
    final target = _uploadTargetLabel(requestBaseUrl);
    _throwIfCancelled(cancellationToken);
    final request = http.AbortableStreamedRequest(
      'PUT',
      Uri.parse(requestBaseUrl).resolve(path),
      abortTrigger: cancellationToken?.whenCancelled,
    );
    request.contentLength = bytes.length;
    request.headers.addAll(
      _buildUploadHeaders(
        mimeType: mimeType,
        sha256: sha256,
        filename: filename,
      ),
    );
    if (requestBaseUrl != _baseUrl) {
      request.headers[HttpHeaders.hostHeader] = Uri.parse(_baseUrl).authority;
    }
    final writeRequest = request.sink
        .addStream(_uploadByteStream(bytes, onProgress))
        .whenComplete(request.sink.close);
    try {
      final response = await _http.send(request);
      await writeRequest;
      _throwIfCancelled(cancellationToken);
      final buffered = await http.Response.fromStream(response);
      _debugMediaUpload(
        'request target=$target path=$path status=${buffered.statusCode} '
        'elapsedMs=${stopwatch.elapsedMilliseconds}',
      );
      return buffered;
    } catch (error) {
      _debugMediaUpload(
        'request target=$target path=$path failed=${error.runtimeType} '
        'elapsedMs=${stopwatch.elapsedMilliseconds}',
      );
      rethrow;
    }
  }

  void _throwIfCancelled(UploadCancellationToken? cancellationToken) {
    if (cancellationToken?.isCancelled ?? false) {
      throw const UploadCancelledException();
    }
  }

  Map<String, String> _buildUploadHeaders({
    required String mimeType,
    required String sha256,
    String? filename,
  }) {
    final headers = <String, String>{
      'Authorization': _buildUploadAuthHeader(sha256),
      'Content-Type': mimeType,
      'X-SHA-256': sha256,
    };
    if (filename != null && filename.isNotEmpty) {
      headers['X-Buzz-Filename'] = base64Url
          .encode(utf8.encode(filename))
          .replaceAll('=', '');
    }
    return headers;
  }

  String _buildUploadAuthHeader(String sha256) {
    final authEvent = _buildUploadAuthEvent(sha256);
    final authJson = authEvent.toJson();
    final encoded = base64Url.encode(utf8.encode(authJson)).replaceAll('=', '');
    return 'Nostr $encoded';
  }

  nostr.Event _buildUploadAuthEvent(String sha256) {
    final nsec = _nsec;
    if (nsec == null || nsec.isEmpty) {
      throw Exception('Cannot upload media: no signing key available');
    }

    final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    if (privkeyHex.isEmpty) {
      throw Exception('Invalid nsec');
    }

    final expiration =
        (_now().millisecondsSinceEpoch ~/ 1000) + _uploadAuthLifetimeSeconds;
    final tags = <List<String>>[
      ['t', 'upload'],
      ['x', sha256],
      ['expiration', '$expiration'],
      if (extractServerAuthority(_baseUrl) case final authority?)
        ['server', authority],
    ];

    return nostr.Event.from(
      kind: _uploadAuthKind,
      content: 'Upload buzz-media',
      tags: tags,
      secretKey: privkeyHex,
      verify: false,
    );
  }

  Future<_PreparedUploadImage> _prepareUploadImage(XFile pickedImage) async {
    final bytes = await pickedImage.readAsBytes();
    final detectedMimeType = _tryDetectImageMimeType(bytes);
    if (detectedMimeType case final mimeType?) {
      return _prepareDetectedUploadImage(bytes, mimeType);
    }

    if (_shouldTranscodePickedImage(pickedImage, bytes)) {
      return _prepareTranscodedUploadImage(bytes);
    }

    throw Exception('unsupported file type');
  }

  Future<_PreparedUploadImage> _prepareDetectedUploadImage(
    Uint8List bytes,
    String mimeType,
  ) async {
    final preparedBytes = await _sanitizeImageBytesIfNeeded(bytes, mimeType);
    return _buildPreparedUploadImage(preparedBytes);
  }

  Future<_PreparedUploadImage> _prepareTranscodedUploadImage(
    Uint8List bytes,
  ) async {
    final transcodedBytes = await _transcodeImageToJpeg(bytes);
    return _buildPreparedUploadImage(transcodedBytes);
  }

  _PreparedUploadImage _buildPreparedUploadImage(Uint8List bytes) {
    return _PreparedUploadImage(
      bytes: bytes,
      mimeType: _detectImageMimeType(bytes),
    );
  }

  Future<Uint8List> _sanitizeImageBytesIfNeeded(
    Uint8List bytes,
    String mimeType,
  ) async {
    if (mimeType == 'image/gif' ||
        (mimeType == 'image/png' && _isAnimatedPng(bytes)) ||
        (mimeType == 'image/webp' && _isAnimatedWebp(bytes))) {
      try {
        return sanitizeAnimatedImageForUpload(bytes, mimeType);
      } on FormatException {
        throw Exception('failed to sanitize image for upload');
      }
    }

    if (!_shouldSanitizePickedImage(mimeType)) {
      return bytes;
    }

    final sanitizedBytes = await _sanitizeImageBytes(bytes, mimeType);
    if (sanitizedBytes.isEmpty) {
      throw Exception('failed to sanitize image for upload');
    }
    return sanitizedBytes;
  }
}

void _debugMediaUpload(String message) {
  if (kDebugMode) debugPrint('Buzz media upload $message');
}

String _uploadTargetLabel(String baseUrl) {
  final uri = Uri.tryParse(baseUrl);
  if (uri == null || uri.authority.isEmpty) return 'invalid-target';
  return '${uri.scheme}://${uri.authority}';
}

List<String> _normalizeUploadBaseUrls(
  String canonicalBaseUrl,
  List<String>? candidates,
) {
  final normalized = <String>[];
  for (final candidate in [...?candidates, canonicalBaseUrl]) {
    final value = candidate.trim();
    if (value.isNotEmpty && !normalized.contains(value)) {
      normalized.add(value);
    }
  }
  return List.unmodifiable(normalized);
}

String _safeAttachmentFilename(String filename) {
  final segments = filename.split(RegExp(r'[/\\]'));
  final basename = segments.isEmpty ? '' : segments.last;
  final sanitized = StringBuffer();
  var byteLength = 0;

  for (final rune in basename.runes) {
    if ((rune >= 0 && rune <= 0x1f) || (rune >= 0x7f && rune <= 0x9f)) {
      continue;
    }

    final character = String.fromCharCode(rune);
    final characterByteLength = utf8.encode(character).length;
    if (byteLength + characterByteLength > 255) break;

    sanitized.write(character);
    byteLength += characterByteLength;
  }

  final safeBasename = sanitized.toString().trim();
  return safeBasename.isEmpty ? 'file' : safeBasename;
}

Stream<List<int>> _uploadByteStream(
  Uint8List bytes,
  ValueChanged<double>? onProgress,
) async* {
  const chunkSize = 64 * 1024;
  onProgress?.call(0);
  if (bytes.isEmpty) {
    onProgress?.call(1);
    return;
  }
  for (var start = 0; start < bytes.length; start += chunkSize) {
    final end = math.min(start + chunkSize, bytes.length);
    yield Uint8List.sublistView(bytes, start, end);
    onProgress?.call(end / bytes.length);
  }
}

String _sha256Hex(Uint8List bytes) {
  final digest = SHA256Digest().process(bytes);
  return digest.map((byte) => byte.toRadixString(16).padLeft(2, '0')).join();
}

String? _tryDetectImageMimeType(Uint8List bytes) {
  try {
    return _detectImageMimeType(bytes);
  } on Exception {
    return null;
  }
}

String _detectImageMimeType(Uint8List bytes) {
  if (_startsWith(bytes, const [0xff, 0xd8, 0xff])) {
    return 'image/jpeg';
  }
  if (_startsWith(bytes, const [
    0x89,
    0x50,
    0x4e,
    0x47,
    0x0d,
    0x0a,
    0x1a,
    0x0a,
  ])) {
    return 'image/png';
  }
  if (_startsWith(bytes, ascii.encode('GIF87a')) ||
      _startsWith(bytes, ascii.encode('GIF89a'))) {
    return 'image/gif';
  }
  if (_startsWith(bytes, ascii.encode('RIFF')) &&
      bytes.length >= 12 &&
      ascii.decode(bytes.sublist(8, 12), allowInvalid: true) == 'WEBP') {
    return 'image/webp';
  }
  throw Exception('unsupported file type');
}

bool _shouldTranscodePickedImage(XFile pickedImage, Uint8List bytes) {
  return _supportsNativeUploadImageProcessing() &&
      (_hasHeicFileExtension(pickedImage) || _looksLikeHeicOrHeif(bytes));
}

bool _isAnimatedPng(Uint8List bytes) {
  if (!_startsWith(bytes, const [
    0x89,
    0x50,
    0x4e,
    0x47,
    0x0d,
    0x0a,
    0x1a,
    0x0a,
  ])) {
    return false;
  }

  var offset = 8;
  while (offset + 12 <= bytes.length) {
    final chunkSize = _readUint32BigEndian(bytes, offset);
    if (offset + 12 + chunkSize > bytes.length) {
      return false;
    }

    if (_matchesAscii(bytes, offset + 4, 'acTL')) {
      return true;
    }

    offset += 12 + chunkSize;
  }

  return false;
}

bool _isAnimatedWebp(Uint8List bytes) {
  if (!_startsWith(bytes, ascii.encode('RIFF')) ||
      bytes.length < 12 ||
      ascii.decode(bytes.sublist(8, 12), allowInvalid: true) != 'WEBP') {
    return false;
  }

  var offset = 12;
  while (offset + 8 <= bytes.length) {
    final chunkSize = _readUint32LittleEndian(bytes, offset + 4);
    final payloadOffset = offset + 8;
    if (payloadOffset + chunkSize > bytes.length) {
      return false;
    }

    if (_matchesAscii(bytes, offset, 'ANIM') ||
        _matchesAscii(bytes, offset, 'ANMF')) {
      return true;
    }
    if (_matchesAscii(bytes, offset, 'VP8X') &&
        chunkSize >= 1 &&
        (bytes[payloadOffset] & 0x02) != 0) {
      return true;
    }

    offset = payloadOffset + chunkSize + (chunkSize.isOdd ? 1 : 0);
  }

  return false;
}

bool _shouldSanitizePickedImage(String mimeType) {
  return _supportsNativeUploadImageProcessing() &&
      (mimeType == 'image/jpeg' ||
          mimeType == 'image/png' ||
          mimeType == 'image/webp');
}

bool _supportsNativeUploadImageProcessing() {
  return switch (defaultTargetPlatform) {
    TargetPlatform.android || TargetPlatform.iOS => true,
    _ => false,
  };
}

bool _hasHeicFileExtension(XFile pickedImage) {
  for (final candidate in [pickedImage.name, pickedImage.path]) {
    final normalizedCandidate = candidate.toLowerCase();
    if (normalizedCandidate.endsWith('.heic') ||
        normalizedCandidate.endsWith('.heif')) {
      return true;
    }
  }
  return false;
}

bool _looksLikeHeicOrHeif(Uint8List bytes) {
  if (bytes.length < 12 || !_matchesAscii(bytes, 4, 'ftyp')) {
    return false;
  }

  final upperBound = bytes.length < 32 ? bytes.length : 32;
  for (var offset = 8; offset + 4 <= upperBound; offset += 4) {
    final brand = ascii.decode(
      bytes.sublist(offset, offset + 4),
      allowInvalid: true,
    );
    if (_heicBrands.contains(brand.toLowerCase())) {
      return true;
    }
  }

  return false;
}

bool _startsWith(Uint8List bytes, List<int> prefix) {
  if (bytes.length < prefix.length) return false;
  for (var i = 0; i < prefix.length; i++) {
    if (bytes[i] != prefix[i]) return false;
  }
  return true;
}

bool _matchesAscii(Uint8List bytes, int offset, String value) {
  final codeUnits = ascii.encode(value);
  if (bytes.length < offset + codeUnits.length) return false;
  for (var i = 0; i < codeUnits.length; i++) {
    if (bytes[offset + i] != codeUnits[i]) return false;
  }
  return true;
}

int _readUint32BigEndian(Uint8List bytes, int offset) {
  return (bytes[offset] << 24) |
      (bytes[offset + 1] << 16) |
      (bytes[offset + 2] << 8) |
      bytes[offset + 3];
}

int _readUint32LittleEndian(Uint8List bytes, int offset) {
  return bytes[offset] |
      (bytes[offset + 1] << 8) |
      (bytes[offset + 2] << 16) |
      (bytes[offset + 3] << 24);
}

Future<Uint8List?> _readPlatformClipboardImage() async {
  return _mediaUploadPlatformChannel.invokeMethod<Uint8List>(
    _readClipboardImageMethod,
  );
}

Future<Uint8List?> _generatePickedVideoPoster(String filePath) {
  return _mediaUploadPlatformChannel.invokeMethod<Uint8List>(
    _generateVideoPosterMethod,
    filePath,
  );
}

Future<String> _transcodePickedVideoToMp4(String filePath) async {
  final result = await _mediaUploadPlatformChannel.invokeMethod<String>(
    _transcodeVideoToMp4Method,
    filePath,
  );
  if (result == null || result.isEmpty) {
    throw Exception('Failed to convert video to MP4.');
  }
  if (defaultTargetPlatform == TargetPlatform.android) {
    final source = File(result);
    final destination = File(
      '$result.faststart-${DateTime.now().microsecondsSinceEpoch}.mp4',
    );
    try {
      await rewriteMp4ForFastStart(source, destination);
      await source.delete();
      return destination.path;
    } catch (_) {
      try {
        await destination.delete();
      } on FileSystemException {
        // Best-effort cleanup; preserve the original platform error.
      }
      rethrow;
    }
  }
  return result;
}

Future<Uint8List> _transcodePickedImageToJpeg(Uint8List bytes) async {
  return _invokeRequiredPlatformBytesMethod(
    _transcodeImageToJpegMethod,
    arguments: bytes,
    errorMessage: 'failed to convert image for upload',
  );
}

Future<Uint8List> _sanitizePickedImageBytes(
  Uint8List bytes,
  String mimeType,
) async {
  return _invokeRequiredPlatformBytesMethod(
    _sanitizeImageForUploadMethod,
    arguments: {'bytes': bytes, 'mimeType': mimeType},
    errorMessage: 'failed to sanitize image for upload',
  );
}

Future<Uint8List> _invokeRequiredPlatformBytesMethod(
  String method, {
  Object? arguments,
  required String errorMessage,
}) async {
  final result = await _mediaUploadPlatformChannel.invokeMethod<Uint8List>(
    method,
    arguments,
  );
  if (result == null || result.isEmpty) {
    throw Exception(errorMessage);
  }
  return result;
}

Future<XFile?> _pickAttachmentFile() async {
  if (defaultTargetPlatform != TargetPlatform.android) {
    return file_selector.openFile();
  }

  final result = await _mediaUploadPlatformChannel
      .invokeMapMethod<String, Object?>(_pickAttachmentFileMethod);
  if (result == null) return null;

  final path = result['path'] as String?;
  final name = result['name'] as String?;
  final mimeType = result['mimeType'] as String?;
  if (path == null || path.isEmpty || name == null || name.isEmpty) {
    throw const FormatException('Android file picker returned invalid data.');
  }
  return XFile(path, name: name, mimeType: mimeType);
}

final mediaUploadServiceProvider = Provider<MediaUploadService>((ref) {
  final config = ref.watch(relayConfigProvider);
  final picker = ImagePicker();
  final service = MediaUploadService(
    baseUrl: config.baseUrl,
    baseUrls: config.httpBaseUrls,
    nsec: config.nsec,
    pickGalleryImage: () => picker.pickImage(
      source: ImageSource.gallery,
      requestFullMetadata: false,
    ),
    pickGalleryImages: () => picker.pickMultiImage(requestFullMetadata: false),
    pickGalleryVideo: () => picker.pickVideo(source: ImageSource.gallery),
    pickAttachmentFile: _pickAttachmentFile,
  );
  ref.onDispose(service.dispose);
  return service;
});

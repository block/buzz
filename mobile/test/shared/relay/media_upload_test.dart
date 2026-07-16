import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:image_picker/image_picker.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/shared/relay/media_auth.dart';
import 'package:buzz/shared/relay/media_upload.dart';

final _pngBytes = Uint8List.fromList([
  0x89,
  0x50,
  0x4e,
  0x47,
  0x0d,
  0x0a,
  0x1a,
  0x0a,
  0x00,
  0x00,
  0x00,
  0x0d,
  0x49,
  0x48,
  0x44,
  0x52,
]);

final _jpegBytes = Uint8List.fromList([
  0xff,
  0xd8,
  0xff,
  0xdb,
  0x00,
  0x43,
  0x00,
  0x01,
]);

final _heicBytes = Uint8List.fromList([
  0x00,
  0x00,
  0x00,
  0x18,
  0x66,
  0x74,
  0x79,
  0x70,
  0x68,
  0x65,
  0x69,
  0x63,
  0x00,
  0x00,
  0x00,
  0x00,
  0x6d,
  0x69,
  0x66,
  0x31,
  0x68,
  0x65,
  0x69,
  0x63,
]);

final _gifBytes = Uint8List.fromList([
  0x47,
  0x49,
  0x46,
  0x38,
  0x39,
  0x61,
  0x01,
  0x00,
  0x01,
  0x00,
]);

final _apngBytes = Uint8List.fromList([
  0x89,
  0x50,
  0x4e,
  0x47,
  0x0d,
  0x0a,
  0x1a,
  0x0a,
  0x00,
  0x00,
  0x00,
  0x08,
  0x61,
  0x63,
  0x54,
  0x4c,
  0x00,
  0x00,
  0x00,
  0x02,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x49,
  0x45,
  0x4e,
  0x44,
  0x00,
  0x00,
  0x00,
  0x00,
]);

final _staticPngWithActlPayloadBytes = Uint8List.fromList([
  0x89,
  0x50,
  0x4e,
  0x47,
  0x0d,
  0x0a,
  0x1a,
  0x0a,
  0x00,
  0x00,
  0x00,
  0x04,
  0x49,
  0x44,
  0x41,
  0x54,
  0x61,
  0x63,
  0x54,
  0x4c,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x49,
  0x45,
  0x4e,
  0x44,
  0x00,
  0x00,
  0x00,
  0x00,
]);

final _animatedWebpBytes = Uint8List.fromList([
  0x52,
  0x49,
  0x46,
  0x46,
  0x16,
  0x00,
  0x00,
  0x00,
  0x57,
  0x45,
  0x42,
  0x50,
  0x56,
  0x50,
  0x38,
  0x58,
  0x0a,
  0x00,
  0x00,
  0x00,
  0x02,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
  0x00,
]);

const _mediaUploadPlatformChannel = MethodChannel('buzz/media_upload');

void _setMockMediaUploadPlatformHandler(
  Future<Object?> Function(MethodCall call)? handler,
) {
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(_mediaUploadPlatformChannel, handler);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUpAll(() {
    _setMockMediaUploadPlatformHandler((call) async {
      switch (call.method) {
        case 'sanitizeImageForUpload':
          final arguments = call.arguments as Map<Object?, Object?>;
          return arguments['bytes'] as Uint8List;
        case 'transcodeImageToJpeg':
          return _jpegBytes;
        default:
          return null;
      }
    });
  });

  tearDownAll(() {
    _setMockMediaUploadPlatformHandler(null);
  });

  group('MediaGetAuthService', () {
    test('signs relay media get requests with a server-scoped token', () {
      final keychain = nostr.Keys.generate();
      final service = MediaGetAuthService(
        baseUrl: 'https://Relay.Example:443',
        nsec: keychain.nsec,
        now: () => DateTime.fromMillisecondsSinceEpoch(1700000000000),
      );

      final headers = service.headersFor(
        'https://relay.example:443/media/${'a' * 64}.jpg',
      );

      final authHeader = headers['Authorization'];
      expect(authHeader, startsWith('Nostr '));
      final encoded = authHeader!.substring('Nostr '.length);
      final decoded = utf8.decode(
        base64Url.decode(base64Url.normalize(encoded)),
      );
      final authEvent = jsonDecode(decoded) as Map<String, dynamic>;
      expect(authEvent['kind'], 24242);
      expect(authEvent['pubkey'], keychain.public);
      expect(authEvent['content'], 'Get buzz-media');
      expect(authEvent['tags'], contains(equals(['t', 'get'])));
      expect(authEvent['tags'], contains(equals(['server', 'relay.example'])));
      expect(authEvent['tags'], contains(equals(['expiration', '1700000600'])));
    });

    test('does not sign non-relay or non-media URLs', () {
      final service = MediaGetAuthService(
        baseUrl: 'https://relay.example',
        nsec: nostr.Keys.generate().nsec,
      );

      expect(
        service.headersFor('https://evil.example/media/${'a' * 64}.jpg'),
        isEmpty,
      );
      expect(service.headersFor('https://relay.example/avatar.png'), isEmpty);
    });

    test('normalizes default ports and rejects path-prefix lookalikes', () {
      final service = MediaGetAuthService(
        baseUrl: 'https://Relay.Example:443',
        nsec: nostr.Keys.generate().nsec,
      );

      expect(
        service.headersFor('https://relay.example/media/${'a' * 64}.jpg'),
        isNotEmpty,
      );
      expect(
        service.headersFor('https://relay.example/media-evil/${'a' * 64}.jpg'),
        isEmpty,
      );
      expect(
        service.headersFor('ftp://relay.example/media/${'a' * 64}.jpg'),
        isEmpty,
      );
    });

    test('does not sign without a key', () {
      final service = MediaGetAuthService(
        baseUrl: 'https://relay.example',
        nsec: null,
      );

      expect(
        service.headersFor('https://relay.example/media/${'a' * 64}.jpg'),
        isEmpty,
      );
    });

    test('does not throw or sign when the stored key is invalid', () {
      final service = MediaGetAuthService(
        baseUrl: 'https://relay.example',
        nsec: 'not-an-nsec',
      );

      expect(
        service.headersFor('https://relay.example/media/${'a' * 64}.jpg'),
        isEmpty,
      );
    });
  });

  group('MediaUploadService', () {
    test('signs Blossom auth and uploads gallery image bytes', () async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;

      http.Request? capturedRequest;
      final client = http_testing.MockClient((request) async {
        capturedRequest = request;
        return http.Response(
          jsonEncode({
            'url': 'https://relay.example/media/test.png',
            'sha256':
                '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
            'size': 16,
            'type': 'image/png',
            'uploaded': 1,
            'thumb': 'https://relay.example/media/test.thumb.jpg',
          }),
          200,
        );
      });

      final service = MediaUploadService(
        baseUrl: 'https://relay.example:8443',
        nsec: nsec,
        httpClient: client,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_pngBytes, name: 'tiny.png'),
        now: () => DateTime.fromMillisecondsSinceEpoch(1_700_000_000_000),
      );

      final descriptor = await service.pickAndUploadImage();

      expect(descriptor, isNotNull);
      expect(descriptor!.type, 'image/png');
      expect(capturedRequest, isNotNull);
      expect(
        capturedRequest!.url.toString(),
        'https://relay.example:8443/media/upload',
      );
      expect(capturedRequest!.headers['Content-Type'], 'image/png');
      expect(capturedRequest!.headers['X-SHA-256'], isNotEmpty);
      expect(capturedRequest!.bodyBytes, _pngBytes);

      final authHeader = capturedRequest!.headers['Authorization'];
      expect(authHeader, isNotNull);
      expect(authHeader, startsWith('Nostr '));
      final encoded = authHeader!.substring('Nostr '.length);
      final decoded = utf8.decode(
        base64Url.decode(base64Url.normalize(encoded)),
      );
      final authEvent = jsonDecode(decoded) as Map<String, dynamic>;
      final tags = (authEvent['tags'] as List<dynamic>)
          .map((tag) => (tag as List<dynamic>).cast<String>())
          .toList();

      expect(authEvent['kind'], 24242);
      expect(authEvent['pubkey'], keychain.public);
      expect(tags, anyElement(equals(<String>['t', 'upload'])));
      expect(
        tags,
        anyElement(
          equals(<String>['x', capturedRequest!.headers['X-SHA-256']!]),
        ),
      );
      expect(tags, anyElement(equals(<String>['expiration', '1700000300'])));
      expect(
        tags,
        anyElement(equals(<String>['server', 'relay.example:8443'])),
      );
    });

    test(
      'checks clipboard image availability through the platform channel',
      () async {
        final invokedMethods = <String>[];
        _setMockMediaUploadPlatformHandler((call) async {
          invokedMethods.add(call.method);
          if (call.method == 'clipboardHasImage') return true;
          return null;
        });
        addTearDown(() {
          _setMockMediaUploadPlatformHandler((call) async {
            switch (call.method) {
              case 'sanitizeImageForUpload':
                final arguments = call.arguments as Map<Object?, Object?>;
                return arguments['bytes'] as Uint8List;
              case 'transcodeImageToJpeg':
                return _jpegBytes;
              default:
                return null;
            }
          });
        });
        final service = MediaUploadService(
          baseUrl: 'https://relay.example',
          nsec: null,
          pickGalleryVideo: () async => null,
          pickGalleryImage: () async => null,
        );

        expect(await service.clipboardHasImage(), isTrue);
        expect(invokedMethods, ['clipboardHasImage']);
      },
    );

    test('reads clipboard image through the platform channel', () async {
      final invokedMethods = <String>[];
      _setMockMediaUploadPlatformHandler((call) async {
        invokedMethods.add(call.method);
        if (call.method == 'readClipboardImage') return _pngBytes;
        if (call.method == 'sanitizeImageForUpload') {
          final arguments = call.arguments as Map<Object?, Object?>;
          return arguments['bytes'] as Uint8List;
        }
        return null;
      });
      addTearDown(() {
        _setMockMediaUploadPlatformHandler((call) async {
          switch (call.method) {
            case 'sanitizeImageForUpload':
              final arguments = call.arguments as Map<Object?, Object?>;
              return arguments['bytes'] as Uint8List;
            case 'transcodeImageToJpeg':
              return _jpegBytes;
            default:
              return null;
          }
        });
      });
      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nostr.Keys.generate().nsec,
        httpClient: http_testing.MockClient(
          (request) async => http.Response(
            jsonEncode({
              'url': 'https://relay.example/media/clipboard.png',
              'sha256':
                  '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
              'size': 16,
              'type': 'image/png',
              'uploaded': 1,
            }),
            200,
          ),
        ),
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async => null,
      );

      final descriptor = await service.readAndUploadClipboardImage();

      expect(invokedMethods.first, 'readClipboardImage');
      expect(descriptor.type, 'image/png');
    });

    test(
      'rejects GIF clipboard bytes through the shared validation path',
      () async {
        final service = MediaUploadService(
          baseUrl: 'https://relay.example',
          nsec: null,
          pickGalleryVideo: () async => null,
          pickGalleryImage: () async => null,
          readClipboardImage: () async => _gifBytes,
        );

        expect(
          service.readAndUploadClipboardImage,
          throwsA(
            isA<Exception>().having(
              (error) => error.toString(),
              'message',
              contains('GIF uploads are not supported on mobile yet'),
            ),
          ),
        );
      },
    );

    test('rejects empty clipboard image bytes', () async {
      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: null,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async => null,
        readClipboardImage: () async => Uint8List(0),
      );

      expect(
        service.readAndUploadClipboardImage,
        throwsA(
          isA<Exception>().having(
            (error) => error.toString(),
            'message',
            contains('Unable to read pasted image'),
          ),
        ),
      );
    });

    test('returns null when the gallery picker is cancelled', () async {
      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: null,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async => null,
      );

      final result = await service.pickAndUploadImage();
      expect(result, isNull);
    });

    test('uses a bracketed IPv6 server tag in Blossom auth', () async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;

      http.Request? capturedRequest;
      final client = http_testing.MockClient((request) async {
        capturedRequest = request;
        return http.Response(
          jsonEncode({
            'url': 'http://[::1]:3000/media/test.png',
            'sha256':
                '2222222222222222222222222222222222222222222222222222222222222222',
            'size': 16,
            'type': 'image/png',
            'uploaded': 1,
          }),
          200,
        );
      });

      final service = MediaUploadService(
        baseUrl: 'http://[::1]:3000',
        nsec: nsec,
        httpClient: client,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_pngBytes, name: 'tiny.png'),
      );

      await service.pickAndUploadImage();

      expect(capturedRequest, isNotNull);
      final authHeader = capturedRequest!.headers['Authorization'];
      expect(authHeader, isNotNull);
      final encoded = authHeader!.substring('Nostr '.length);
      final decoded = utf8.decode(
        base64Url.decode(base64Url.normalize(encoded)),
      );
      final authEvent = jsonDecode(decoded) as Map<String, dynamic>;
      final tags = (authEvent['tags'] as List<dynamic>)
          .map((tag) => (tag as List<dynamic>).cast<String>())
          .toList();

      expect(tags, anyElement(equals(<String>['server', '[::1]:3000'])));
    });

    test('transcodes HEIC gallery files on iOS before upload', () async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() {
        debugDefaultTargetPlatformOverride = previousPlatform;
      });

      Uint8List? transcodedInput;
      http.Request? capturedRequest;
      final client = http_testing.MockClient((request) async {
        capturedRequest = request;
        return http.Response(
          jsonEncode({
            'url': 'https://relay.example/media/test.jpg',
            'sha256':
                'fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210',
            'size': _jpegBytes.length,
            'type': 'image/jpeg',
            'uploaded': 1,
          }),
          200,
        );
      });

      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        httpClient: client,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_heicBytes, name: 'photo.heic'),
        transcodeImageToJpeg: (bytes) async {
          transcodedInput = bytes;
          return _jpegBytes;
        },
      );

      final descriptor = await service.pickAndUploadImage();

      expect(descriptor, isNotNull);
      expect(descriptor!.type, 'image/jpeg');
      expect(transcodedInput, _heicBytes);
      expect(capturedRequest, isNotNull);
      expect(capturedRequest!.headers['Content-Type'], 'image/jpeg');
      expect(capturedRequest!.bodyBytes, _jpegBytes);
    });

    test('sanitizes iOS JPEG gallery files before upload', () async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() {
        debugDefaultTargetPlatformOverride = previousPlatform;
      });

      Uint8List? sanitizedInput;
      String? sanitizedMimeType;
      http.Request? capturedRequest;
      final client = http_testing.MockClient((request) async {
        capturedRequest = request;
        return http.Response(
          jsonEncode({
            'url': 'https://relay.example/media/test.jpg',
            'sha256':
                'abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd',
            'size': _jpegBytes.length,
            'type': 'image/jpeg',
            'uploaded': 1,
          }),
          200,
        );
      });

      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        httpClient: client,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_jpegBytes, name: 'photo.jpg'),
        sanitizeImageBytes: (bytes, mimeType) async {
          sanitizedInput = bytes;
          sanitizedMimeType = mimeType;
          return _jpegBytes;
        },
      );

      final descriptor = await service.pickAndUploadImage();

      expect(descriptor, isNotNull);
      expect(descriptor!.type, 'image/jpeg');
      expect(sanitizedInput, _jpegBytes);
      expect(sanitizedMimeType, 'image/jpeg');
      expect(capturedRequest, isNotNull);
      expect(capturedRequest!.headers['Content-Type'], 'image/jpeg');
      expect(capturedRequest!.bodyBytes, _jpegBytes);
    });

    test('transcodes HEIC gallery files on Android before upload', () async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      addTearDown(() {
        debugDefaultTargetPlatformOverride = previousPlatform;
      });

      Uint8List? transcodedInput;
      http.Request? capturedRequest;
      final client = http_testing.MockClient((request) async {
        capturedRequest = request;
        return http.Response(
          jsonEncode({
            'url': 'https://relay.example/media/test.jpg',
            'sha256':
                '1234512345123451234512345123451234512345123451234512345123451234',
            'size': _jpegBytes.length,
            'type': 'image/jpeg',
            'uploaded': 1,
          }),
          200,
        );
      });

      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        httpClient: client,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_heicBytes, name: 'photo.heic'),
        transcodeImageToJpeg: (bytes) async {
          transcodedInput = bytes;
          return _jpegBytes;
        },
      );

      final descriptor = await service.pickAndUploadImage();

      expect(descriptor, isNotNull);
      expect(descriptor!.type, 'image/jpeg');
      expect(transcodedInput, _heicBytes);
      expect(capturedRequest, isNotNull);
      expect(capturedRequest!.headers['Content-Type'], 'image/jpeg');
      expect(capturedRequest!.bodyBytes, _jpegBytes);
    });

    test('sanitizes Android JPEG gallery files before upload', () async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      addTearDown(() {
        debugDefaultTargetPlatformOverride = previousPlatform;
      });

      Uint8List? sanitizedInput;
      String? sanitizedMimeType;
      http.Request? capturedRequest;
      final client = http_testing.MockClient((request) async {
        capturedRequest = request;
        return http.Response(
          jsonEncode({
            'url': 'https://relay.example/media/test.jpg',
            'sha256':
                'abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd',
            'size': _jpegBytes.length,
            'type': 'image/jpeg',
            'uploaded': 1,
          }),
          200,
        );
      });

      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        httpClient: client,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_jpegBytes, name: 'photo.jpg'),
        sanitizeImageBytes: (bytes, mimeType) async {
          sanitizedInput = bytes;
          sanitizedMimeType = mimeType;
          return _jpegBytes;
        },
      );

      final descriptor = await service.pickAndUploadImage();

      expect(descriptor, isNotNull);
      expect(descriptor!.type, 'image/jpeg');
      expect(sanitizedInput, _jpegBytes);
      expect(sanitizedMimeType, 'image/jpeg');
      expect(capturedRequest, isNotNull);
      expect(capturedRequest!.headers['Content-Type'], 'image/jpeg');
      expect(capturedRequest!.bodyBytes, _jpegBytes);
    });

    test('sanitizes Android PNG gallery files before upload', () async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;
      final previousPlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      addTearDown(() {
        debugDefaultTargetPlatformOverride = previousPlatform;
      });

      Uint8List? sanitizedInput;
      String? sanitizedMimeType;
      http.Request? capturedRequest;
      final client = http_testing.MockClient((request) async {
        capturedRequest = request;
        return http.Response(
          jsonEncode({
            'url': 'https://relay.example/media/test.png',
            'sha256':
                '9999999999999999999999999999999999999999999999999999999999999999',
            'size': _pngBytes.length,
            'type': 'image/png',
            'uploaded': 1,
          }),
          200,
        );
      });

      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        httpClient: client,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_pngBytes, name: 'photo.png'),
        sanitizeImageBytes: (bytes, mimeType) async {
          sanitizedInput = bytes;
          sanitizedMimeType = mimeType;
          return _pngBytes;
        },
      );

      final descriptor = await service.pickAndUploadImage();

      expect(descriptor, isNotNull);
      expect(descriptor!.type, 'image/png');
      expect(sanitizedInput, _pngBytes);
      expect(sanitizedMimeType, 'image/png');
      expect(capturedRequest, isNotNull);
      expect(capturedRequest!.headers['Content-Type'], 'image/png');
      expect(capturedRequest!.bodyBytes, _pngBytes);
    });

    test('rejects GIF gallery files before upload', () async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;

      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_gifBytes, name: 'animated.gif'),
      );

      expect(
        service.pickAndUploadImage(),
        throwsA(
          isA<Exception>().having(
            (error) => error.toString(),
            'message',
            contains('GIF uploads are not supported on mobile yet'),
          ),
        ),
      );
    });

    test('rejects animated PNG gallery files before upload', () async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;

      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        httpClient: http_testing.MockClient(
          (request) async => http.Response('{}', 200),
        ),
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_apngBytes, name: 'animated.png'),
      );

      expect(
        service.pickAndUploadImage(),
        throwsA(
          isA<Exception>().having(
            (error) => error.toString(),
            'message',
            contains('Animated PNG uploads are not supported on mobile yet'),
          ),
        ),
      );
    });

    test('uploads static PNG when acTL appears only in chunk payload', () async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;

      http.Request? capturedRequest;
      final client = http_testing.MockClient((request) async {
        capturedRequest = request;
        return http.Response(
          jsonEncode({
            'url': 'https://relay.example/media/static.png',
            'sha256':
                '1111111111111111111111111111111111111111111111111111111111111111',
            'size': _staticPngWithActlPayloadBytes.length,
            'type': 'image/png',
            'uploaded': 1,
          }),
          200,
        );
      });

      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        httpClient: client,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_staticPngWithActlPayloadBytes, name: 'static.png'),
      );

      final descriptor = await service.pickAndUploadImage();

      expect(descriptor, isNotNull);
      expect(descriptor!.type, 'image/png');
      expect(capturedRequest, isNotNull);
      expect(capturedRequest!.headers['Content-Type'], 'image/png');
      expect(capturedRequest!.bodyBytes, _staticPngWithActlPayloadBytes);
    });

    test('rejects animated WebP gallery files before upload', () async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;

      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        httpClient: http_testing.MockClient(
          (request) async => http.Response('{}', 200),
        ),
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async =>
            XFile.fromData(_animatedWebpBytes, name: 'animated.webp'),
      );

      expect(
        service.pickAndUploadImage(),
        throwsA(
          isA<Exception>().having(
            (error) => error.toString(),
            'message',
            contains('Animated WebP uploads are not supported on mobile yet'),
          ),
        ),
      );
    });

    test('rejects unsupported gallery files before upload', () async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;

      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: nsec,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async => XFile.fromData(
          Uint8List.fromList(utf8.encode('not an image')),
          name: 'note.txt',
        ),
      );

      expect(
        service.pickAndUploadImage(),
        throwsA(
          isA<Exception>().having(
            (error) => error.toString(),
            'message',
            contains('unsupported file type'),
          ),
        ),
      );
    });
  });

  group('pickAndUploadVideo', () {
    // Helper: build ftyp header bytes for a given brand.
    Uint8List buildFtypHeader(String brand) {
      final bytes = Uint8List(32);
      bytes[3] = 32;
      bytes[4] = 0x66;
      bytes[5] = 0x74;
      bytes[6] = 0x79;
      bytes[7] = 0x70;
      final brandBytes = ascii.encode(brand);
      for (var i = 0; i < 4 && i < brandBytes.length; i++) {
        bytes[8 + i] = brandBytes[i];
      }
      return bytes;
    }

    // Helper: write bytes to a temp file, return its XFile.
    Future<(XFile, File)> writeTempVideo(Uint8List bytes, String name) async {
      final dir = await Directory.systemTemp.createTemp('video_test_');
      final file = File('${dir.path}/$name');
      await file.writeAsBytes(bytes);
      return (XFile(file.path), file);
    }

    test('rebuilds an existing MP4 container before upload', () async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;
      var transcodeCalled = false;
      final client = http_testing.MockClient((request) async {
        return http.Response(
          jsonEncode({
            'url': 'https://relay.example/media/test.mp4',
            'sha256':
                '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
            'size': 32,
            'type': 'video/mp4',
            'uploaded': 1,
          }),
          200,
        );
      });

      final mp4Bytes = buildFtypHeader('isom');
      final (xfile, tempFile) = await writeTempVideo(mp4Bytes, 'clip.mp4');
      try {
        final service = MediaUploadService(
          baseUrl: 'https://relay.example',
          nsec: nsec,
          httpClient: client,
          pickGalleryVideo: () async => xfile,
          pickGalleryImage: () async => null,
          transcodeVideoToMp4: (path) async {
            transcodeCalled = true;
            final outDir = await Directory.systemTemp.createTemp('transcode_');
            final outFile = File('${outDir.path}/out.mp4');
            await outFile.writeAsBytes(buildFtypHeader('isom'));
            return outFile.path;
          },
          now: () => DateTime.fromMillisecondsSinceEpoch(1_700_000_000_000),
        );

        final descriptor = await service.pickAndUploadVideo();
        expect(descriptor, isNotNull);
        expect(descriptor!.type, 'video/mp4');
        expect(transcodeCalled, isTrue);
      } finally {
        await tempFile.parent.delete(recursive: true);
      }
    });

    test('transcodes non-MP4 container before uploading', () async {
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;
      var transcodeCalled = false;
      final client = http_testing.MockClient((request) async {
        expect(request.headers['Content-Type'], 'video/mp4');
        return http.Response(
          jsonEncode({
            'url': 'https://relay.example/media/test.mp4',
            'sha256':
                '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
            'size': 32,
            'type': 'video/mp4',
            'uploaded': 1,
          }),
          200,
        );
      });

      // QuickTime container ('qt  ' brand) — needs transcoding.
      final movBytes = buildFtypHeader('qt  ');
      final (xfile, tempFile) = await writeTempVideo(movBytes, 'clip.mov');
      try {
        final service = MediaUploadService(
          baseUrl: 'https://relay.example',
          nsec: nsec,
          httpClient: client,
          pickGalleryVideo: () async => xfile,
          pickGalleryImage: () async => null,
          transcodeVideoToMp4: (path) async {
            transcodeCalled = true;
            // Mock transcoding: write an "MP4" file
            final outDir = await Directory.systemTemp.createTemp('transcode_');
            final outFile = File('${outDir.path}/out.mp4');
            await outFile.writeAsBytes(buildFtypHeader('isom'));
            return outFile.path;
          },
          now: () => DateTime.fromMillisecondsSinceEpoch(1_700_000_000_000),
        );

        final descriptor = await service.pickAndUploadVideo();
        expect(descriptor, isNotNull);
        expect(descriptor!.type, 'video/mp4');
        expect(transcodeCalled, isTrue);
      } finally {
        await tempFile.parent.delete(recursive: true);
      }
    });

    test('returns null when video picker is cancelled', () async {
      final service = MediaUploadService(
        baseUrl: 'https://relay.example',
        nsec: null,
        pickGalleryVideo: () async => null,
        pickGalleryImage: () async => null,
      );

      final result = await service.pickAndUploadVideo();
      expect(result, isNull);
    });

    test('rejects videos over 100MB', () async {
      // Create a temp file with 101MB of zeros.
      final dir = await Directory.systemTemp.createTemp('video_size_test_');
      final file = File('${dir.path}/huge.mp4');
      final raf = await file.open(mode: FileMode.write);
      await raf.truncate(101 * 1024 * 1024);
      await raf.close();

      try {
        final service = MediaUploadService(
          baseUrl: 'https://relay.example',
          nsec: null,
          pickGalleryVideo: () async => XFile(file.path),
          pickGalleryImage: () async => null,
        );

        await expectLater(
          () => service.pickAndUploadVideo(),
          throwsA(
            isA<Exception>().having(
              (e) => e.toString(),
              'message',
              contains('too large'),
            ),
          ),
        );
      } finally {
        await dir.delete(recursive: true);
      }
    });
  });
}

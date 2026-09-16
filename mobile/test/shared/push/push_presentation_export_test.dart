import 'dart:async';
import 'dart:isolate';

import 'package:buzz/shared/push/push_presentation_cache.dart';
import 'package:buzz/shared/push/push_presentation_export_recovery.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

const _channel = MethodChannel('buzz/push');
const _secret =
    '0000000000000000000000000000000000000000000000000000000000000001';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final calls = <MethodCall>[];
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;

  setUp(() {
    calls.clear();
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    pushPresentationCacheError.value = null;
    pushPresentationExportError.value = null;
    messenger.setMockMethodCallHandler(_channel, (call) async {
      calls.add(call);
      return null;
    });
  });
  tearDown(() {
    messenger.setMockMethodCallHandler(_channel, null);
    debugDefaultTargetPlatformOverride = null;
    pushPresentationCacheError.value = null;
    pushPresentationExportError.value = null;
  });

  for (final profiles in [true, false]) {
    for (final failedChunk in [0, 1]) {
      test(
        'native chunk transient failure retries ${profiles ? "profiles" : "channels"} chunk $failedChunk in FIFO slot',
        () async {
          final limit = profiles ? 256 : 512;
          final observations = ReceivePort();
          addTearDown(observations.close);
          var workerReads = 0;
          final caller = Isolate.current.debugName;
          observations.listen((name) {
            if (name != caller) workerReads++;
          });
          final events = [
            for (var i = 0; i <= limit; i++)
              _signed(
                profiles ? 0 : 39000,
                100,
                channelID: 'channel-$i',
                secretKey: profiles
                    ? (i + 1).toRadixString(16).padLeft(64, '0')
                    : _secret,
              ),
          ];
          events[0] = _ObservedVerificationEvent(
            events[0],
            observations.sendPort,
          );
          final memberships = profiles
              ? <NostrEvent>[]
              : [
                  for (var i = 0; i <= limit; i++)
                    _signed(39002, 100, channelID: 'channel-$i'),
                ];
          var failed = false;
          final successfulChunks = <Map>[];
          messenger.setMockMethodCallHandler(_channel, (call) async {
            calls.add(call);
            final args = call.arguments as Map;
            if (args['communityId'] == 'following-community') return null;
            final chunk =
                (args[profiles ? 'events' : 'metadataEvents'] as List).length ==
                    limit
                ? 0
                : 1;
            if (chunk == failedChunk && !failed) {
              failed = true;
              throw PlatformException(
                code: profiles
                    ? 'profile_cache_failed'
                    : 'channel_cache_failed',
              );
            }
            successfulChunks.add(args);
            return null;
          });
          final exported = PushPresentationExportRecovery().export(
            () => profiles
                ? cacheBuzzPushProfileEvents('original-community', events)
                : cacheBuzzPushChannelEvents(
                    'original-community',
                    events,
                    memberships,
                  ),
          );
          final following = cacheBuzzPushProfileEvents('following-community', [
            _signed(0, 200),
          ]);
          expect(await exported, isTrue);
          await following;
          expect(calls.map((call) => call.arguments['communityId']), [
            'original-community',
            'original-community',
            'original-community',
            'following-community',
          ]);
          expect(
            calls[failedChunk + 1].arguments,
            equals(calls[failedChunk].arguments),
            reason: 'retry must reuse the exact failed verified chunk',
          );
          expect(successfulChunks, hasLength(2));
          expect([
            for (final chunk in successfulChunks)
              ...chunk[profiles ? 'events' : 'metadataEvents'] as List,
          ], unorderedEquals(events.map((event) => event.toJson())));
          if (!profiles) {
            expect([
              for (final chunk in successfulChunks)
                ...chunk['membershipEvents'] as List,
            ], unorderedEquals(memberships.map((event) => event.toJson())));
          }
          await Future<void>.delayed(Duration.zero);
          expect(
            workerReads,
            1,
            reason: 'native retries must not repeat signature verification',
          );
          expect(pushPresentationExportError.value, isNull);
        },
      );
    }
  }

  for (final profiles in [true, false]) {
    test(
      'permanent first ${profiles ? "profile" : "channel"} chunk failure exhausts retries and stops export',
      () async {
        final limit = profiles ? 256 : 512;
        final events = [
          for (var i = 0; i <= limit; i++)
            _signed(
              profiles ? 0 : 39000,
              100,
              channelID: 'channel-$i',
              secretKey: profiles
                  ? (i + 1).toRadixString(16).padLeft(64, '0')
                  : _secret,
            ),
        ];
        final memberships = profiles
            ? <NostrEvent>[]
            : [
                for (var i = 0; i <= limit; i++)
                  _signed(39002, 100, channelID: 'channel-$i'),
              ];
        messenger.setMockMethodCallHandler(_channel, (call) async {
          calls.add(call);
          if (call.arguments['communityId'] == 'failed-community') {
            throw PlatformException(
              code: profiles ? 'profile_cache_failed' : 'channel_cache_failed',
            );
          }
          return null;
        });
        final recovery = PushPresentationExportRecovery();
        final succeeded = await recovery.export(
          () => profiles
              ? cacheBuzzPushProfileEvents('failed-community', events)
              : cacheBuzzPushChannelEvents(
                  'failed-community',
                  events,
                  memberships,
                ),
        );
        expect(succeeded, isFalse);
        expect(
          calls,
          hasLength(6),
          reason: 'initial attempt plus five retries, with no later chunk',
        );
        expect(
          pushPresentationCacheError.value,
          contains(profiles ? 'profile_cache_failed' : 'channel_cache_failed'),
        );
        final terminal = pushPresentationExportError.value;
        expect(
          terminal,
          contains(profiles ? 'profile_cache_failed' : 'channel_cache_failed'),
        );
        await cacheBuzzPushProfileEvents('following-community', [
          _signed(0, 200),
        ]);
        expect(calls, hasLength(7));
        for (final retry in calls.take(6)) {
          expect(retry.arguments, equals(calls.first.arguments));
        }
        expect(calls.last.arguments['communityId'], 'following-community');
        expect(pushPresentationExportError.value, terminal);
      },
    );
  }

  test('native retry budget is shared across all chunks', () async {
    final events = [
      for (var i = 1; i <= 257; i++)
        _signed(0, 100, secretKey: i.toRadixString(16).padLeft(64, '0')),
    ];
    messenger.setMockMethodCallHandler(_channel, (call) async {
      calls.add(call);
      final rows = call.arguments['events'] as List;
      if (calls.length == 1 || rows.length == 1) {
        throw PlatformException(code: 'profile_cache_failed');
      }
      return null;
    });
    expect(
      await PushPresentationExportRecovery().export(
        () => cacheBuzzPushProfileEvents('community', events),
      ),
      isFalse,
    );
    expect(calls, hasLength(7));
    expect(calls.map((call) => (call.arguments['events'] as List).length), [
      256,
      256,
      1,
      1,
      1,
      1,
      1,
    ]);
    expect(pushPresentationExportError.value, contains('profile_cache_failed'));
  });

  test(
    'native argument errors are terminal without persistence retries',
    () async {
      messenger.setMockMethodCallHandler(_channel, (call) async {
        calls.add(call);
        throw PlatformException(code: 'invalid_arguments');
      });
      expect(
        await PushPresentationExportRecovery().export(
          () => cacheBuzzPushProfileEvents('community', [_signed(0, 100)]),
        ),
        isFalse,
      );
      expect(calls, hasLength(1));
      expect(pushPresentationExportError.value, contains('invalid_arguments'));
    },
  );

  test('unavailable native bridge remains an intentional no-op', () async {
    messenger.setMockMethodCallHandler(_channel, (call) async {
      calls.add(call);
      throw MissingPluginException('synthetic non-Runner embedding');
    });
    expect(
      await PushPresentationExportRecovery().export(
        () => cacheBuzzPushProfileEvents('community', [_signed(0, 100)]),
      ),
      isTrue,
    );
    expect(calls, hasLength(1));
    expect(pushPresentationCacheError.value, isNull);
    expect(pushPresentationExportError.value, isNull);
  });

  for (final profiles in [true, false]) {
    test(
      'large ${profiles ? "profile" : "channel"} exports fit native bounds',
      () async {
        final limit = profiles ? 256 : 512;
        final events = [
          for (var i = 0; i <= limit; i++)
            _signed(
              profiles ? 0 : 39000,
              100,
              channelID: 'channel-$i',
              secretKey: profiles
                  ? (i + 1).toRadixString(16).padLeft(64, '0')
                  : _secret,
            ),
        ];
        // Deliberately reverse roster order: each chunk must pair by channel ID.
        final memberships = profiles
            ? <NostrEvent>[]
            : [
                for (var i = limit; i >= 0; i--)
                  _signed(39002, 100, channelID: 'channel-$i'),
              ];
        final received = <dynamic>[];
        final receivedMemberships = <dynamic>[];
        messenger.setMockMethodCallHandler(_channel, (call) async {
          final args = call.arguments as Map;
          if (args['communityId'] == 'following-community') {
            calls.add(call);
            return null;
          }
          final metadata = args[profiles ? 'events' : 'metadataEvents'] as List;
          final rosters = profiles
              ? <dynamic>[]
              : args['membershipEvents'] as List;
          if (metadata.length > limit || rosters.length > limit) {
            throw PlatformException(code: 'invalid_arguments');
          }
          expect(args['communityId'], 'bounded-community');
          if (!profiles) {
            String id(dynamic event) =>
                (event['tags'] as List).firstWhere((tag) => tag[0] == 'd')[1]
                    as String;
            expect(rosters.map(id).toSet(), metadata.map(id).toSet());
          }
          received.addAll(metadata);
          receivedMemberships.addAll(rosters);
          calls.add(call);
          return null;
        });
        final largeExport = profiles
            ? cacheBuzzPushProfileEvents('bounded-community', events)
            : cacheBuzzPushChannelEvents(
                'bounded-community',
                events,
                memberships,
              );
        final followingExport = cacheBuzzPushProfileEvents(
          'following-community',
          [_signed(0, 200)],
        );
        await Future.wait([largeExport, followingExport]);
        expect(pushPresentationCacheError.value, isNull);
        expect(calls.map((call) => call.arguments['communityId']), [
          'bounded-community',
          'bounded-community',
          'following-community',
        ]);
        expect(
          received,
          unorderedEquals(events.map((event) => event.toJson())),
        );
        expect(
          receivedMemberships,
          unorderedEquals(memberships.map((event) => event.toJson())),
        );
      },
    );
  }

  for (final kind in [0, 39000]) {
    test(
      'kind $kind verifies off the calling isolate before native export',
      () async {
        final verification = Completer<void>();
        final observations = ReceivePort();
        addTearDown(observations.close);
        final callerIsolateName = Isolate.current.debugName;
        observations.listen((name) {
          if (name != callerIsolateName && !verification.isCompleted) {
            verification.complete();
          }
        });
        final event = _ObservedVerificationEvent(
          _signed(kind, 100),
          observations.sendPort,
        );
        var timerRan = false;
        Timer.run(() => timerRan = true);
        if (kind == 0) {
          await cacheBuzzPushProfileEvents('community', [event]);
        } else {
          await cacheBuzzPushChannelEvents('community', [event], []);
        }
        // The signal originates from reading the real signature in the worker.
        // Restoring synchronous verification cannot produce it. This is a
        // liveness timeout, not a device-dependent elapsed-time performance gate.
        await verification.future.timeout(const Duration(seconds: 5));
        expect(timerRan, isTrue);
        expect(calls.single.method, 'syncPushSnapshot');
        expect(calls.single.arguments['communityId'], 'community');
        final key = kind == 0 ? 'events' : 'metadataEvents';
        expect(calls.single.arguments[key], [event.toJson()]);
      },
    );
  }

  test(
    'profile export retains older valid candidates and signed payloads',
    () async {
      final older = _signed(0, 100);
      final newestValid = _signed(0, 200);
      await cacheBuzzPushProfileEvents('profiles-community', [
        older,
        _tampered(_signed(0, 300)),
        newestValid,
        _signed(39000, 400),
      ]);
      expect(calls.single.arguments, {
        'section': 'profiles',
        'communityId': 'profiles-community',
        'events': [newestValid.toJson()],
      });
    },
  );

  test(
    'channel export preserves valid fallback and membership scope pairing',
    () async {
      final metadata = _signed(39000, 200);
      final membership = _signed(39002, 200);
      await cacheBuzzPushChannelEvents(
        'channels-community',
        [
          _signed(39000, 100),
          _tampered(_signed(39000, 300)),
          metadata,
          _signed(39000, 400, channelID: 'unpaired'),
        ],
        [_tampered(_signed(39002, 300)), membership, _signed(39002, 100)],
      );
      expect(calls.single.arguments, {
        'section': 'channels',
        'communityId': 'channels-community',
        'metadataEvents': [metadata.toJson()],
        'membershipEvents': [membership.toJson()],
      });
    },
  );

  test(
    'native handoff and completion retain FIFO across sections and communities',
    () async {
      final firstEntered = Completer<void>();
      final releaseFirst = Completer<void>();
      var firstReleased = false;
      var overlappingHandoff = false;
      messenger.setMockMethodCallHandler(_channel, (call) async {
        calls.add(call);
        if (calls.length == 1) {
          firstEntered.complete();
          await releaseFirst.future;
          firstReleased = true;
        } else {
          overlappingHandoff |= !firstReleased;
        }
        return null;
      });
      final completed = <String>[];
      final first = cacheBuzzPushProfileEvents('first', [
        _signed(0, 100),
      ]).then((_) => completed.add('first'));
      await firstEntered.future;
      final second = cacheBuzzPushChannelEvents('second', [
        _signed(39000, 100),
      ], []).then((_) => completed.add('second'));
      final third = cacheBuzzPushProfileEvents('third', [
        _signed(0, 200),
      ]).then((_) => completed.add('third'));
      try {
        await Future<void>.delayed(Duration.zero);
        expect(completed, isEmpty);
        expect(calls.length, 1);
      } finally {
        releaseFirst.complete();
        await Future.wait([first, second, third]);
      }
      expect(overlappingHandoff, isFalse);
      expect(completed, ['first', 'second', 'third']);
      expect(calls.map((call) => call.arguments['communityId']), [
        'first',
        'second',
        'third',
      ]);
    },
  );

  test(
    'saturation rejects explicitly and admitted exports drain before recovery',
    () async {
      final entered = Completer<void>();
      final release = Completer<void>();
      messenger.setMockMethodCallHandler(_channel, (call) async {
        calls.add(call);
        if (calls.length == 1) {
          entered.complete();
          await release.future;
        }
        return null;
      });
      final event = _signed(0, 100);
      final admitted = [
        cacheBuzzPushProfileEvents('0', [event]),
      ];
      await entered.future;
      try {
        for (var i = 1; i < 8; i++) {
          admitted.add(cacheBuzzPushProfileEvents('$i', [event]));
        }
        await expectLater(
          cacheBuzzPushProfileEvents('rejected', [
            event,
          ]).timeout(const Duration(seconds: 5)),
          throwsStateError,
        );
        expect(calls.length, 1);
      } finally {
        release.complete();
        await Future.wait(admitted);
      }
      expect(calls.map((call) => call.arguments['communityId']), [
        for (var i = 0; i < 8; i++) '$i',
      ]);
      await cacheBuzzPushProfileEvents('recovered', [event]);
      expect(calls.last.arguments['communityId'], 'recovered');
      expect(calls.length, 9);
    },
  );

  test(
    'worker submission failure propagates and releases the next export',
    () async {
      final unsendable = ReceivePort();
      addTearDown(unsendable.close);
      final failed = cacheBuzzPushProfileEvents('failed', [
        _UnsendableEvent(_signed(0, 100), unsendable),
      ]);
      final failure = expectLater(failed, throwsArgumentError);
      final next = cacheBuzzPushProfileEvents('next', [_signed(0, 200)]);
      await failure;
      await next.timeout(const Duration(seconds: 5));
      expect(calls.length, 1);
      expect(calls.single.arguments['communityId'], 'next');
    },
  );
}

NostrEvent _signed(
  int kind,
  int createdAt, {
  String channelID = 'channel',
  String secretKey = _secret,
}) => NostrEvent.fromJson(
  nostr.Event.from(
    kind: kind,
    createdAt: createdAt,
    content: kind == 0 ? '{"name":"Synthetic profile"}' : '',
    tags: kind == 0
        ? []
        : [
            ['d', channelID],
          ],
    secretKey: secretKey,
  ).toMap(),
);

NostrEvent _tampered(NostrEvent event) => NostrEvent(
  id: event.id,
  pubkey: event.pubkey,
  createdAt: event.createdAt,
  kind: event.kind,
  tags: event.tags,
  content: 'Tampered',
  sig: event.sig,
);

class _ObservedVerificationEvent extends NostrEvent {
  final SendPort observations;

  _ObservedVerificationEvent(NostrEvent event, this.observations)
    : super(
        id: event.id,
        pubkey: event.pubkey,
        createdAt: event.createdAt,
        kind: event.kind,
        tags: event.tags,
        content: event.content,
        sig: event.sig,
      );

  @override
  String get sig {
    observations.send(Isolate.current.debugName);
    return super.sig;
  }
}

class _UnsendableEvent extends NostrEvent {
  final ReceivePort port;

  _UnsendableEvent(NostrEvent event, this.port)
    : super(
        id: event.id,
        pubkey: event.pubkey,
        createdAt: event.createdAt,
        kind: event.kind,
        tags: event.tags,
        content: event.content,
        sig: event.sig,
      );
}

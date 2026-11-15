part of 'channels_provider_test.dart';

const _channelC = '33333333-3333-4333-8333-333333333333';

/// Launches of the app over one saved directory file, with a clock the test
/// sets.
class _SavedDirectoryHarness {
  _SavedDirectoryHarness(this.file, this.session);

  final File file;
  _FakeRelaySession session;
  final sessions = <_FakeRelaySession>[];
  var now = DateTime.utc(2026, 1, 1);
  late ProviderContainer container;

  static Future<_SavedDirectoryHarness> create(
    _FakeRelaySession session,
  ) async {
    final root = await Directory.systemTemp.createTemp('saved-directory');
    // A save still running at teardown can recreate a file mid-delete.
    addTearDown(() async {
      for (var attempt = 0; ; attempt++) {
        try {
          return await root.delete(recursive: true);
        } on FileSystemException {
          if (attempt == 5) rethrow;
          await Future<void>.delayed(const Duration(milliseconds: 50));
        }
      }
    });
    return _SavedDirectoryHarness(File('${root.path}/directory.json'), session);
  }

  int get nowSecs => now.millisecondsSinceEpoch ~/ 1000;

  ChannelsNotifier get channels => container.read(channelsProvider.notifier);

  /// Starts the app on [session] over [storage], or the saved file.
  Future<void> launch({OpenChannelDirectoryStorage? storage}) async {
    sessions.add(session);
    container = _buildContainer(
      session: session,
      overrides: [
        openChannelDirectoryStorageProvider.overrideWithValue(
          storage ?? OpenChannelDirectoryStorage(Future.value(file)),
        ),
        channelDirectoryClockProvider.overrideWithValue(() => now),
      ],
    );
    addTearDown(container.dispose);
    await container.read(channelsProvider.future);
  }

  /// Ends this launch and starts the next on a fresh session serving the same
  /// metadata.
  Future<void> relaunch() async {
    container.dispose();
    session = _FakeRelaySession(
      memberships: session.memberships,
      metadata: session.metadata,
    );
    await launch();
  }

  /// The `since` of the first page of each directory read; a read pages on
  /// with `until`.
  List<int?> reads() => [
    for (final session in sessions)
      for (final filter in session.directoryQueryFilters)
        if (filter.until == null) filter.since,
  ];

  /// The names of the list's channels the user has not joined.
  List<String> unjoined() => [
    for (final channel in container.read(channelsProvider).requireValue)
      if (!channel.isMember) channel.name,
  ]..sort();

  List<String> saved() {
    if (!file.existsSync()) return const [];
    final json = jsonDecode(file.readAsStringSync()) as Map;
    return [
      for (final meta in (json['metas'] as List).cast<Map>())
        for (final tag in (meta['tags'] as List).cast<List>())
          if (tag.first == 'name') tag[1] as String,
    ]..sort();
  }

  /// Saving runs behind the list update, on real file I/O.
  Future<void> expectSaved(List<String> expected) async {
    for (var i = 0; i < 200 && !listEquals(saved(), expected); i++) {
      await Future<void>.delayed(const Duration(milliseconds: 10));
    }
    expect(saved(), expected);
  }

  /// Opens the directory as a reader does and lets its reads settle.
  Future<void> open() async {
    await channels.ensureDirectoryLoaded();
    await _settle();
  }
}

void _savedDirectoryCases() {
  test('the saved directory holds only discoverable channels', () async {
    final app = await _SavedDirectoryHarness.create(
      _FakeRelaySession(
        memberships: [
          _membership(_channelB, 'me'),
          _membership(_channelC, 'me'),
          _membership(_channelD, 'me'),
        ],
        metadata: [
          _meta(id: _channelA, name: 'general', createdAt: 100),
          _meta(id: _channelB, name: 'staff', visibility: 'private'),
          _meta(id: _channelC, name: 'quiet', unlisted: true),
          _meta(id: _channelD, name: 'pair', channelType: 'dm'),
        ],
      ),
    );
    await app.launch();

    // The relay's directory read also returns the user's own private,
    // unlisted and DM channels; none of them is saved.
    await app.open();
    expect(app.unjoined(), ['general']);
    await app.expectSaved(['general']);

    // A catch-up that finds a saved channel turned private drops it.
    app.now = app.now.add(const Duration(minutes: 1));
    app.session.metadata = [
      ...app.session.metadata.skip(1),
      _meta(
        id: _channelA,
        name: 'general',
        visibility: 'private',
        createdAt: 200,
      ),
    ];
    await app.open();
    expect(app.reads().last, isNotNull);
    expect(app.unjoined(), isEmpty);
    await app.expectSaved([]);
  });

  test('a catch-up starts before the last read, by device clock', () async {
    final app = await _SavedDirectoryHarness.create(
      _FakeRelaySession(memberships: const []),
    );
    final start = app.nowSecs;
    // The device clock runs five minutes ahead of the relay's, and a burst of
    // joins has stamped the channel ten minutes ahead of the relay.
    final relayStart = start - 300;
    app.session.metadata = [
      _meta(id: _channelA, name: 'busy', createdAt: relayStart + 600),
    ];
    await app.launch();
    await app.open();
    expect(app.unjoined(), ['busy']);

    // Two minutes on, a channel created meanwhile carries an earlier stamp
    // than both the newest saved one and the device's read time, and the
    // catch-up still finds it.
    app.now = app.now.add(const Duration(minutes: 2));
    app.session.metadata = [
      ...app.session.metadata,
      _meta(id: _channelB, name: 'fresh', createdAt: relayStart + 120),
    ];
    await app.open();
    expect(app.unjoined(), ['busy', 'fresh']);
    expect(app.reads().last, start - relayCrossDeviceSkewSeconds);
    await app.expectSaved(['busy', 'fresh']);

    // After a relaunch the saved read time still bounds the cursor, now from
    // the catch-up's start.
    await app.relaunch();
    app.now = app.now.add(const Duration(minutes: 1));
    await app.open();
    expect(app.reads().last, start + 120 - relayCrossDeviceSkewSeconds);
  });

  test('a read dated in the future by a clock since set back is due', () async {
    final app = await _SavedDirectoryHarness.create(
      _FakeRelaySession(
        memberships: const [],
        metadata: [_meta(id: _channelA, name: 'general', createdAt: 100)],
      ),
    );
    // Saved with the clock a year ahead.
    app.now = DateTime.utc(2027, 1, 1);
    await app.launch();
    await app.open();
    await app.expectSaved(['general']);

    // Set back, the next reader still checks, and the full read is due.
    app.now = DateTime.utc(2026, 1, 1);
    await app.open();
    expect(app.reads(), [null, null]);

    // Saved a year ahead again, a relaunch with the clock set back reads the
    // whole directory rather than catching up.
    app.now = DateTime.utc(2027, 1, 1);
    app.session.metadata = [
      _meta(id: _channelA, name: 'renamed', createdAt: 200),
    ];
    await app.open();
    expect(app.reads().last, isNull);
    await app.expectSaved(['renamed']);
    await app.relaunch();
    app.now = DateTime.utc(2026, 1, 1);
    await app.open();
    expect(app.reads().last, isNull);
  });

  test(
    'a reader opening before the session connects checks on connect',
    () async {
      final app = await _SavedDirectoryHarness.create(
        _FakeRelaySession(
          memberships: const [],
          metadata: [_meta(id: _channelA, name: 'general', createdAt: 100)],
        ),
      );
      await app.launch();
      await app.open();
      await app.expectSaved(['general']);

      // Days later, the app relaunches and search opens before the session
      // connects: the saved copy serves at once.
      app.now = app.now.add(const Duration(days: 3));
      app.session.metadata = [
        _meta(id: _channelB, name: 'fresh', createdAt: 200),
      ];
      await app.relaunch();
      app.session.setStatus(SessionStatus.connecting);
      await app.open();
      expect(app.unjoined(), ['general']);
      final reads = app.reads().length;

      // Connecting runs the check the reader missed: here, the overdue full
      // read.
      app.session.setStatus(SessionStatus.connected);
      await _settle();
      expect(app.reads().skip(reads), [null]);
      expect(app.unjoined(), ['fresh']);
    },
  );

  test('a restore during an account switch keeps the old list out', () async {
    final app = await _SavedDirectoryHarness.create(
      _FakeRelaySession(
        memberships: [_membership(_channelB, 'me')],
        metadata: [
          _meta(id: _channelA, name: 'general', createdAt: 100),
          _meta(id: _channelB, name: 'staff', visibility: 'private'),
          _meta(id: _channelC, name: 'quiet', unlisted: true),
        ],
      ),
    );
    await app.launch();
    await app.open();
    await app.expectSaved(['general']);

    // Another account takes over, and its member list is slow to load. A
    // reader restoring the saved directory meanwhile must not publish the
    // previous account's list, with its private channel, as this account's.
    app.session.memberships = [_membership(_channelC, _otherPk)];
    final gate = app.session.metadataGate = Completer<void>();
    app.container.read(_testPubkeyProvider.notifier).set(_otherPk);
    await _settle();
    await app.channels.ensureDirectoryLoaded();
    await _settle();
    expect(app.container.read(channelsProvider).isLoading, isTrue);

    // The load lands with the restored directory merged in.
    gate.complete();
    await app.container.read(channelsProvider.future);
    await _settle();
    expect(
      [
        for (final channel in app.container.read(channelsProvider).requireValue)
          channel.name,
      ]..sort(),
      ['general', 'quiet'],
    );
  });

  test('restoring the saved directory leaves a running load alone', () async {
    final app = await _SavedDirectoryHarness.create(
      _FakeRelaySession(
        memberships: const [],
        metadata: [_meta(id: _channelA, name: 'general', createdAt: 100)],
      ),
    );
    await app.launch();
    await app.open();
    await app.expectSaved(['general']);

    // On the next launch the saved file is slow to open, and a retry starts
    // a full read meanwhile.
    app.container.dispose();
    app.now = app.now.add(const Duration(minutes: 5));
    app.session = _FakeRelaySession(
      memberships: const [],
      metadata: [_meta(id: _channelB, name: 'fresh', createdAt: 50)],
    );
    final file = Completer<File?>();
    await app.launch(storage: OpenChannelDirectoryStorage(file.future));
    final opened = app.channels.ensureDirectoryLoaded();
    await _settle();
    app.session.pauseNextDirectoryQuery();
    final retry = app.channels.retryDirectory(directoryOnly: true);
    await app.session.nextDirectoryQueryStarted;
    ChannelDirectoryLoadStatus status() =>
        app.container.read(channelDirectoryLoadStatusProvider).status;
    expect(status(), ChannelDirectoryLoadStatus.loading);

    // The restore neither reports the read finished nor displaces it with a
    // catch-up.
    file.complete(app.file);
    await opened;
    await _settle();
    expect(status(), ChannelDirectoryLoadStatus.loading);
    app.session.resumePausedDirectoryQuery();
    await retry;
    await _settle();
    expect(status(), ChannelDirectoryLoadStatus.loaded);
    expect(app.reads().last, isNull);
    expect(app.unjoined(), ['fresh']);
    await app.expectSaved(['fresh']);
  });

  test(
    'a full read in flight does not bring back a channel dropped on open',
    () async {
      final app = await _SavedDirectoryHarness.create(
        _FakeRelaySession(
          memberships: const [],
          metadata: [
            _meta(id: _channelA, name: 'general', createdAt: 100),
            _meta(id: _channelB, name: 'fresh', createdAt: 100),
          ],
        ),
      );
      await app.launch();
      await app.open();
      expect(app.unjoined(), ['fresh', 'general']);

      // A full read starts, then opening a channel finds it gone.
      app.session.pauseNextDirectoryQuery();
      final retry = app.channels.retryDirectory(directoryOnly: true);
      await app.session.nextDirectoryQueryStarted;
      app.session.metadata = [app.session.metadata.last];
      await expectLater(
        app.container.read(channelDetailsProvider(_channelA).future),
        throwsException,
      );
      await _settle();
      expect(app.unjoined(), ['fresh']);

      // The read's response predates the drop, so it does not bring it back.
      app.session.resumePausedDirectoryQuery();
      await retry;
      await _settle();
      expect(app.unjoined(), ['fresh']);
      await app.expectSaved(['fresh']);
    },
  );
}

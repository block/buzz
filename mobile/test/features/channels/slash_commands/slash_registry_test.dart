import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/slash_commands/slash_channel_refs.dart';
import 'package:buzz/features/channels/slash_commands/slash_command.dart';
import 'package:buzz/features/channels/slash_commands/slash_executor.dart';
import 'package:buzz/features/channels/slash_commands/slash_registry.dart';
import 'package:buzz/shared/slash_commands/slash_errors.dart';
import 'package:buzz/shared/slash_commands/slash_tokenizer.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

Future<void> _noop(SlashCommandContext ctx) async {}

SlashCommand _command(
  String name, {
  List<String> aliases = const [],
  SlashScope scope = SlashScope.any,
  bool Function(SlashVisibilityContext)? isVisible,
  SlashRun run = _noop,
}) => SlashCommand(
  name: name,
  aliases: aliases,
  description: '$name description',
  usage: '/$name <arg>',
  scope: scope,
  isVisible: isVisible,
  run: run,
);

class _Harness {
  final WidgetRef ref;
  final BuildContext context;
  final List<String> shown = [];

  _Harness(this.ref, this.context);

  SlashVisibilityContext visibility({bool isDm = false}) =>
      SlashVisibilityContext(
        ref: ref,
        channelId: 'c1',
        channel: null,
        isDm: isDm,
        inThread: false,
      );

  SlashCommandContext commandContext({bool isDm = false, String args = ''}) =>
      SlashCommandContext(
        ref: ref,
        context: context,
        channelId: 'c1',
        channel: null,
        isDm: isDm,
        threadHeadId: null,
        rootId: null,
        args: args,
        tokens: tokenizeSlashArgs(args),
        mentions: const [],
        channels: const [],
        unresolvedChannelNames: const [],
        sendMessage: (_, {mentionPubkeys = const []}) async {},
        showMessage: shown.add,
      );
}

Future<_Harness> _pumpHarness(WidgetTester tester) async {
  late _Harness harness;
  await tester.pumpWidget(
    ProviderScope(
      child: MaterialApp(
        home: Consumer(
          builder: (context, ref, _) {
            harness = _Harness(ref, context);
            return const SizedBox();
          },
        ),
      ),
    ),
  );
  return harness;
}

Channel _channel(String name, {String type = 'stream'}) => Channel(
  id: 'id-$name',
  name: name,
  channelType: type,
  visibility: 'open',
  description: '',
  createdBy: 'x',
  createdAt: DateTime(2025),
  memberCount: 1,
);

void main() {
  group('the registry', () {
    test('composes every group without a name or alias collision', () {
      expect(slashRegistry.lookup('shrug'), isNotNull);
      for (final command in slashRegistry.commands) {
        for (final name in [command.name, ...command.aliases]) {
          expect(name, matches(RegExp(r'^[a-z0-9_-]+$')), reason: '/$name');
        }
      }
    });

    test('refuses a name claimed twice', () {
      expect(
        () => SlashRegistry([
          _command('remove', aliases: ['kick']),
          _command('kick'),
        ]),
        throwsStateError,
      );
    });

    testWidgets('suggests by name prefix, alphabetically, never by alias', (
      tester,
    ) async {
      final harness = await _pumpHarness(tester);
      final registry = SlashRegistry([
        _command('status'),
        _command('away'),
        _command('active'),
        _command('remove', aliases: ['kick', 'away-alias']),
        _command('who', aliases: ['members']),
      ]);
      List<String> names(String query, {bool isDm = false}) => [
        for (final c in registry.suggest(query, harness.visibility(isDm: isDm)))
          c.name,
      ];

      expect(names(''), ['active', 'away', 'remove', 'status', 'who']);
      expect(names('a'), ['active', 'away']);
      expect(names('A'), ['active', 'away']);
      expect(names('st'), ['status']);
      expect(names('tus'), isEmpty, reason: 'no fuzzy matching');
      expect(names('kick'), isEmpty, reason: 'hidden alias');
      expect(names('mem'), isEmpty, reason: 'hidden alias');
      expect(names('away-alias'), isEmpty, reason: 'hidden alias');
      expect(registry.lookup('kick')?.name, 'remove');
      expect(registry.recognize('/MEMBERS')?.command.name, 'who');
    });

    testWidgets('hides commands by scope and by visibility predicate', (
      tester,
    ) async {
      final harness = await _pumpHarness(tester);
      var admin = false;
      final registry = SlashRegistry([
        _command('away'),
        _command('leave', scope: SlashScope.channel),
        _command('archive', isVisible: (_) => admin),
      ]);
      List<String> names({bool isDm = false}) => [
        for (final c in registry.suggest('', harness.visibility(isDm: isDm)))
          c.name,
      ];

      expect(names(), ['away', 'leave']);
      expect(names(isDm: true), ['away']);
      admin = true;
      expect(names(), ['archive', 'away', 'leave']);
      expect(
        registry.recognize('/leave')?.command.name,
        'leave',
        reason: 'recognition ignores context and visibility',
      );
    });
  });

  group('executeSlashCommand', () {
    const cases = <(String, Object? Function(), bool, String?)>[
      ('succeeds', _ok, true, null),
      ('a bare usage error', _usage, false, 'Usage: /rename <arg>'),
      (
        'a usage error with a reason',
        _usageReason,
        false,
        'Pick one\nUsage: /rename',
      ),
      ('an expected failure', _failure, false, 'Not allowed'),
      ('an unexpected error', _unexpected, false, 'boom'),
      ('a community switch', _stale, false, '/rename not run'),
    ];
    for (final (label, error, ok, message) in cases) {
      testWidgets(label, (tester) async {
        final harness = await _pumpHarness(tester);
        final command = _command(
          'rename',
          run: (_) async {
            final thrown = error();
            if (thrown != null) throw thrown;
          },
        );

        final result = await executeSlashCommand(
          command,
          harness.commandContext(),
        );

        expect(result, ok);
        if (message == null) {
          expect(harness.shown, isEmpty);
        } else {
          expect(harness.shown.single, startsWith(message));
        }
      });
    }

    testWidgets('refuses a channel command in a DM without running it', (
      tester,
    ) async {
      final harness = await _pumpHarness(tester);
      var ran = false;
      final command = _command(
        'leave',
        scope: SlashScope.channel,
        run: (_) async => ran = true,
      );

      final result = await executeSlashCommand(
        command,
        harness.commandContext(isDm: true),
      );

      expect(result, isFalse);
      expect(ran, isFalse);
      expect(harness.shown.single, '/leave only works in channels');
    });
  });

  group('resolveSlashChannelRefs', () {
    final visible = [
      _channel('general'),
      _channel('Design-Team'),
      _channel('alice-bob', type: 'dm'),
    ];
    const cases = <(String, String, List<String>, List<String>)>[
      ('none', 'hello', [], []),
      ('one', '#general', ['general'], []),
      ('case-insensitive', '#design-team', ['Design-Team'], []),
      ('in order', '#design-team #general', ['Design-Team', 'general'], []),
      ('repeated', '#general #general', ['general'], []),
      ('unknown', '#nope', [], ['nope']),
      ('a DM never matches', '#alice-bob', [], ['alice-bob']),
      ('mixed', 'x #general #nope', ['general'], ['nope']),
      ('not mid-word', 'a#general', [], []),
    ];
    for (final (label, args, found, missing) in cases) {
      test(label, () {
        final refs = resolveSlashChannelRefs(args, visible);
        expect([for (final c in refs.channels) c.name], found);
        expect(refs.unresolved, missing);
      });
    }
  });
}

Object? _ok() => null;
Object? _usage() => const SlashUsageError();
Object? _usageReason() => const SlashUsageError('Pick one');
Object? _failure() => const SlashCommandFailure('Not allowed');
Object? _unexpected() => Exception('boom');
Object? _stale() => StateError('switched');

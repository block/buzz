import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/workspace/workspace.dart';
import 'package:buzz/shared/workspace/workspace_provider.dart';
import 'package:buzz/shared/workspace/workspace_storage.dart';

import 'workspace_storage_test.dart';

void main() {
  late FakeSecureStorage fakeSecure;
  late WorkspaceStorage workspaceStorage;
  late ProviderContainer container;

  setUp(() {
    fakeSecure = FakeSecureStorage();
    workspaceStorage = WorkspaceStorage(secure: fakeSecure);
  });

  tearDown(() => container.dispose());

  ProviderContainer createContainer() {
    return ProviderContainer(
      overrides: [workspaceStorageProvider.overrideWithValue(workspaceStorage)],
    );
  }

  group('WorkspaceListNotifier', () {
    test('loads empty list initially', () async {
      container = createContainer();
      final workspaces = await container.read(workspaceListProvider.future);
      expect(workspaces, isEmpty);
    });

    test('addWorkspace adds to list', () async {
      container = createContainer();
      await container.read(workspaceListProvider.future);

      final ws = Workspace.create(
        name: 'Test',
        relayUrl: 'https://test.example.com',
      );
      await container.read(workspaceListProvider.notifier).addWorkspace(ws);

      final workspaces = await container.read(workspaceListProvider.future);
      expect(workspaces, hasLength(1));
      expect(workspaces.first.name, 'Test');
    });

    test('removeWorkspace removes from list', () async {
      container = createContainer();
      await container.read(workspaceListProvider.future);

      final ws = Workspace.create(
        name: 'Test',
        relayUrl: 'https://test.example.com',
      );
      await container.read(workspaceListProvider.notifier).addWorkspace(ws);
      await container
          .read(workspaceListProvider.notifier)
          .removeWorkspace(ws.id);

      final workspaces = await container.read(workspaceListProvider.future);
      expect(workspaces, isEmpty);
    });

    test('renameWorkspace updates name', () async {
      container = createContainer();
      await container.read(workspaceListProvider.future);

      final ws = Workspace.create(
        name: 'Original',
        relayUrl: 'https://test.example.com',
      );
      await container.read(workspaceListProvider.notifier).addWorkspace(ws);
      await container
          .read(workspaceListProvider.notifier)
          .renameWorkspace(ws.id, 'Renamed');

      final workspaces = await container.read(workspaceListProvider.future);
      expect(workspaces.first.name, 'Renamed');
    });

    test('switchWorkspace updates active ID', () async {
      container = createContainer();
      await container.read(workspaceListProvider.future);

      final ws1 = Workspace.create(
        name: 'One',
        relayUrl: 'https://one.example.com',
      );
      final ws2 = Workspace.create(
        name: 'Two',
        relayUrl: 'https://two.example.com',
      );

      final notifier = container.read(workspaceListProvider.notifier);
      await notifier.addWorkspace(ws1);
      await notifier.addWorkspace(ws2);
      await notifier.switchWorkspace(ws2.id);

      final activeId = await workspaceStorage.loadActiveId();
      expect(activeId, ws2.id);
    });
  });

  group('activeWorkspaceProvider', () {
    test('returns null when no workspaces', () async {
      container = createContainer();
      final active = await container.read(activeWorkspaceProvider.future);
      expect(active, isNull);
    });

    test('returns workspace matching active ID', () async {
      container = createContainer();
      await container.read(workspaceListProvider.future);

      final ws = Workspace.create(
        name: 'Test',
        relayUrl: 'https://test.example.com',
      );
      final notifier = container.read(workspaceListProvider.notifier);
      await notifier.addWorkspace(ws);
      await notifier.switchWorkspace(ws.id);

      final active = await container.read(activeWorkspaceProvider.future);
      expect(active, isNotNull);
      expect(active!.id, ws.id);
      expect(active.name, 'Test');
    });

    test('falls back to first workspace if active ID is invalid', () async {
      container = createContainer();
      await container.read(workspaceListProvider.future);

      final ws = Workspace.create(
        name: 'Fallback',
        relayUrl: 'https://test.example.com',
      );
      final notifier = container.read(workspaceListProvider.notifier);
      await notifier.addWorkspace(ws);

      // Set an invalid active ID.
      await workspaceStorage.saveActiveId('nonexistent-id');

      // Re-read — should fall back.
      container.invalidate(activeWorkspaceProvider);
      final active = await container.read(activeWorkspaceProvider.future);
      expect(active, isNotNull);
      expect(active!.id, ws.id);
    });
  });
}

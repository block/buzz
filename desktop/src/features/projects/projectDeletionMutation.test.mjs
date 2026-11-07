import assert from "node:assert/strict";
import test from "node:test";

import {
  MutationObserver,
  QueryClient,
  QueryObserver,
} from "@tanstack/react-query";
import {
  projectDeletionMutationOptions,
  projectsQueryKey,
  repositoryDeletionMutationOptions,
} from "./projectDeletionMutation.ts";

const project = {
  id: "30621:owner:platform",
  name: "Platform",
};

const repo = {
  id: "30617:owner:repo1",
  repoAddress: "30617:owner:repo1",
  name: "Repo 1",
};

const repo2 = {
  id: "30617:owner:repo2",
  repoAddress: "30617:owner:repo2",
  name: "Repo 2",
};

const explicitProject = {
  id: "30621:owner:proj",
  name: "Explicit Project",
  legacy: false,
  repositories: [repo, repo2],
  repositoryAddresses: [repo.repoAddress, repo2.repoAddress],
};

const legacyProject = {
  id: "30617:owner:repo1",
  name: "Legacy Project",
  legacy: true,
  repositories: [repo],
  repositoryAddresses: [repo.repoAddress],
};

test("lost deletion acknowledgement refetches and removes the stale project", async () => {
  let fetchCount = 0;
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const queryObserver = new QueryObserver(queryClient, {
    queryKey: projectsQueryKey,
    queryFn: async () => {
      fetchCount += 1;
      return fetchCount === 1 ? [project] : [];
    },
  });
  const unsubscribeQuery = queryObserver.subscribe(() => {});
  await queryObserver.refetch();
  assert.deepEqual(queryClient.getQueryData(projectsQueryKey), [project]);

  const mutationObserver = new MutationObserver(
    queryClient,
    projectDeletionMutationOptions(queryClient, async () => {
      throw new Error(
        "Could not confirm whether the project was deleted. Projects were refreshed.",
      );
    }),
  );
  await assert.rejects(mutationObserver.mutate(project), /Could not confirm/);

  assert.equal(fetchCount, 2);
  assert.deepEqual(queryClient.getQueryData(projectsQueryKey), []);
  unsubscribeQuery();
});

test("successful deletion removes the project before refetch", async () => {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false } },
  });
  queryClient.setQueryData(projectsQueryKey, [project]);
  const mutationObserver = new MutationObserver(
    queryClient,
    projectDeletionMutationOptions(queryClient, async () => {}),
  );

  await mutationObserver.mutate(project);

  assert.deepEqual(queryClient.getQueryData(projectsQueryKey), []);
});

test("successful repository deletion removes the repository from project cache", async () => {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false } },
  });
  queryClient.setQueryData(projectsQueryKey, [explicitProject]);
  const mutationObserver = new MutationObserver(
    queryClient,
    repositoryDeletionMutationOptions(queryClient, async () => {}),
  );

  await mutationObserver.mutate(repo);

  const updatedProjects = queryClient.getQueryData(projectsQueryKey);
  assert.equal(updatedProjects.length, 1);
  assert.deepEqual(updatedProjects[0].repositories, [repo2]);
  assert.deepEqual(updatedProjects[0].repositoryAddresses, [repo2.repoAddress]);
});

test("successful repository deletion removes legacy project when sole repository is deleted", async () => {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false } },
  });
  queryClient.setQueryData(projectsQueryKey, [legacyProject]);
  const mutationObserver = new MutationObserver(
    queryClient,
    repositoryDeletionMutationOptions(queryClient, async () => {}),
  );

  await mutationObserver.mutate(repo);

  const updatedProjects = queryClient.getQueryData(projectsQueryKey);
  assert.deepEqual(updatedProjects, []);
});

test("lost repository deletion acknowledgement refetches queries on settled", async () => {
  let fetchCount = 0;
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const queryObserver = new QueryObserver(queryClient, {
    queryKey: projectsQueryKey,
    queryFn: async () => {
      fetchCount += 1;
      return fetchCount === 1 ? [legacyProject] : [];
    },
  });
  const unsubscribeQuery = queryObserver.subscribe(() => {});
  await queryObserver.refetch();
  assert.deepEqual(queryClient.getQueryData(projectsQueryKey), [legacyProject]);

  const mutationObserver = new MutationObserver(
    queryClient,
    repositoryDeletionMutationOptions(queryClient, async () => {
      throw new Error(
        "Could not confirm whether the repository was deleted. Repositories were refreshed.",
      );
    }),
  );
  await assert.rejects(mutationObserver.mutate(repo), /Could not confirm/);

  assert.equal(fetchCount, 2);
  assert.deepEqual(queryClient.getQueryData(projectsQueryKey), []);
  unsubscribeQuery();
});

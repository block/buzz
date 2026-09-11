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
} from "./projectDeletionMutation.ts";

const project = {
  id: "30621:owner:platform",
  name: "Platform",
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

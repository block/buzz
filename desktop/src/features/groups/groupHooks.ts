import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import {
  createUserGroup,
  deleteUserGroup,
  listUserGroups,
  subscribeToUserGroups,
  updateUserGroup,
  userGroupIdFromSnapshot,
  type CreateUserGroupInput,
  type UpdateUserGroupInput,
  type UserGroup,
} from "@/shared/api/relayGroups";

export const groupsQueryKey = ["groups"] as const;

export function useGroupsQuery() {
  return useQuery({
    queryKey: groupsQueryKey,
    queryFn: listUserGroups,
    staleTime: 30_000,
  });
}

export function useGroupsLiveUpdates() {
  const queryClient = useQueryClient();

  React.useEffect(() => {
    let active = true;
    let unsubscribe: (() => Promise<void>) | undefined;

    void subscribeToUserGroups((event) => {
      if (!userGroupIdFromSnapshot(event)) return;
      void queryClient.invalidateQueries({ queryKey: groupsQueryKey });
    })
      .then((stop) => {
        if (active) {
          unsubscribe = stop;
        } else {
          void stop();
        }
      })
      .catch(() => {
        void queryClient.invalidateQueries({ queryKey: groupsQueryKey });
      });

    return () => {
      active = false;
      void unsubscribe?.();
    };
  }, [queryClient]);
}

export function useCreateGroupMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: CreateUserGroupInput) => createUserGroup(input),
    onMutate: async (input) => {
      await queryClient.cancelQueries({ queryKey: groupsQueryKey });
      const previous = queryClient.getQueryData<UserGroup[]>(groupsQueryKey);
      queryClient.setQueryData<UserGroup[]>(groupsQueryKey, (current = []) => [
        input,
        ...current.filter((group) => group.id !== input.id),
      ]);
      return { previous };
    },
    onError: (_error, _input, context) => {
      queryClient.setQueryData(groupsQueryKey, context?.previous);
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: groupsQueryKey });
    },
  });
}

export function useUpdateGroupMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: UpdateUserGroupInput) => updateUserGroup(input),
    onMutate: async ({ next }) => {
      await queryClient.cancelQueries({ queryKey: groupsQueryKey });
      const previous = queryClient.getQueryData<UserGroup[]>(groupsQueryKey);
      queryClient.setQueryData<UserGroup[]>(groupsQueryKey, (current = []) =>
        current.map((group) => (group.id === next.id ? next : group)),
      );
      return { previous };
    },
    onError: (_error, _input, context) => {
      queryClient.setQueryData(groupsQueryKey, context?.previous);
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: groupsQueryKey });
    },
  });
}

export function useDeleteGroupMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: deleteUserGroup,
    onMutate: async (id) => {
      await queryClient.cancelQueries({ queryKey: groupsQueryKey });
      const previous = queryClient.getQueryData<UserGroup[]>(groupsQueryKey);
      queryClient.setQueryData<UserGroup[]>(groupsQueryKey, (current = []) =>
        current.filter((group) => group.id !== id),
      );
      return { previous };
    },
    onError: (_error, _id, context) => {
      queryClient.setQueryData(groupsQueryKey, context?.previous);
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: groupsQueryKey });
    },
  });
}

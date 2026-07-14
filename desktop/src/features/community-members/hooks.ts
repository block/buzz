import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  addRelayMember,
  changeRelayMemberRole,
  getMyRelayMembership,
  listRelayMembers,
  removeRelayMember,
} from "@/shared/api/relayMembers";
import type { RelayMember } from "@/shared/api/types";

export const relayMembersQueryKey = ["relayMembers"] as const;
export const myRelayMembershipQueryKey = ["myRelayMembership"] as const;

export function useRelayMembersQuery(enabled = true) {
  return useQuery({
    enabled,
    queryKey: relayMembersQueryKey,
    queryFn: listRelayMembers,
    staleTime: 30_000,
  });
}

export function useMyRelayMembershipQuery() {
  return useQuery({
    queryKey: myRelayMembershipQueryKey,
    queryFn: getMyRelayMembership,
    staleTime: 60_000,
  });
}

export function useAddRelayMemberMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ pubkey, role }: { pubkey: string; role: string }) =>
      addRelayMember(pubkey, role),
    onMutate: async ({ pubkey, role }) => {
      await queryClient.cancelQueries({ queryKey: relayMembersQueryKey });
      const previous =
        queryClient.getQueryData<RelayMember[]>(relayMembersQueryKey);

      queryClient.setQueryData<RelayMember[]>(relayMembersQueryKey, (old) => [
        ...(old ?? []),
        {
          pubkey,
          role: role as RelayMember["role"],
          addedBy: null,
          createdAt: new Date().toISOString(),
        },
      ]);

      return { previous };
    },
    onError: (_err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(relayMembersQueryKey, context.previous);
      }
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: relayMembersQueryKey }),
        queryClient.invalidateQueries({ queryKey: myRelayMembershipQueryKey }),
      ]);
    },
  });
}

export function useRemoveRelayMemberMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (pubkey: string) => removeRelayMember(pubkey),
    onMutate: async (pubkey) => {
      await queryClient.cancelQueries({ queryKey: relayMembersQueryKey });
      const previous =
        queryClient.getQueryData<RelayMember[]>(relayMembersQueryKey);

      queryClient.setQueryData<RelayMember[]>(relayMembersQueryKey, (old) =>
        old?.filter((m) => m.pubkey.toLowerCase() !== pubkey.toLowerCase()),
      );

      return { previous };
    },
    onError: (_err, _pubkey, context) => {
      if (context?.previous) {
        queryClient.setQueryData(relayMembersQueryKey, context.previous);
      }
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: relayMembersQueryKey }),
        queryClient.invalidateQueries({ queryKey: myRelayMembershipQueryKey }),
      ]);
    },
  });
}

export function useChangeRelayMemberRoleMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      pubkey,
      role,
      newRole,
    }: {
      pubkey: string;
      role?: string;
      newRole?: string;
    }) => changeRelayMemberRole(pubkey, role ?? newRole ?? "member"),
    onMutate: async ({ pubkey, role, newRole }) => {
      const nextRole = (role ?? newRole ?? "member") as RelayMember["role"];
      await queryClient.cancelQueries({ queryKey: relayMembersQueryKey });
      const previous =
        queryClient.getQueryData<RelayMember[]>(relayMembersQueryKey);

      queryClient.setQueryData<RelayMember[]>(relayMembersQueryKey, (old) =>
        old?.map((m) =>
          m.pubkey.toLowerCase() === pubkey.toLowerCase()
            ? { ...m, role: nextRole }
            : m,
        ),
      );

      return { previous };
    },
    onError: (_err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(relayMembersQueryKey, context.previous);
      }
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: relayMembersQueryKey }),
        queryClient.invalidateQueries({ queryKey: myRelayMembershipQueryKey }),
      ]);
    },
  });
}

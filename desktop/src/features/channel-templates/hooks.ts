import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { useFocusedRefetchInterval } from "@/shared/lib/useDocumentVisible";

import {
  createChannelTemplate,
  deleteChannelTemplate,
  duplicateChannelTemplate,
  listChannelTemplates,
  updateChannelTemplate,
} from "@/shared/api/tauriChannelTemplates";
import type {
  ChannelTemplate,
  CreateChannelTemplateInput,
  UpdateChannelTemplateInput,
} from "@/shared/api/types";

export const channelTemplatesQueryKey = ["channel-templates"] as const;

export function useChannelTemplatesQuery() {
  const refetchInterval = useFocusedRefetchInterval(30_000);

  return useQuery({
    queryKey: channelTemplatesQueryKey,
    queryFn: listChannelTemplates,
    staleTime: 30_000,
    refetchInterval,
    refetchOnWindowFocus: true,
  });
}

export function useCreateChannelTemplateMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: CreateChannelTemplateInput) =>
      createChannelTemplate(input),
    onSuccess: (created) => {
      queryClient.setQueryData<ChannelTemplate[]>(
        channelTemplatesQueryKey,
        (current) => {
          const next = current ?? [];
          return [
            created,
            ...next.filter((template) => template.id !== created.id),
          ];
        },
      );
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({
        queryKey: channelTemplatesQueryKey,
      });
    },
  });
}

export function useUpdateChannelTemplateMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: UpdateChannelTemplateInput) =>
      updateChannelTemplate(input),
    onSettled: async () => {
      await queryClient.invalidateQueries({
        queryKey: channelTemplatesQueryKey,
      });
    },
  });
}

export function useDeleteChannelTemplateMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id: string) => deleteChannelTemplate(id),
    onSettled: async () => {
      await queryClient.invalidateQueries({
        queryKey: channelTemplatesQueryKey,
      });
    },
  });
}

export function useDuplicateChannelTemplateMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id: string) => duplicateChannelTemplate(id),
    onSuccess: (created) => {
      queryClient.setQueryData<ChannelTemplate[]>(
        channelTemplatesQueryKey,
        (current) => {
          const next = current ?? [];
          return [
            created,
            ...next.filter((template) => template.id !== created.id),
          ];
        },
      );
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({
        queryKey: channelTemplatesQueryKey,
      });
    },
  });
}

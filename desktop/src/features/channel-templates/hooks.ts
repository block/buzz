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

/** Keeps focused polling at the established 30-second cadence. */
export const CHANNEL_TEMPLATES_REFETCH_INTERVAL_MS = 30_000;
/** Suppresses the focus refetch until channel template data is genuinely stale.
 * Templates change rarely; mutations cover all writes with push-invalidation. */
export const CHANNEL_TEMPLATES_FOCUS_STALE_TIME_MS = 5 * 60_000;

/** Focus-refetch policy for the channel templates query; consumed by focusRefetchPolicy.test.mjs. */
export const channelTemplatesFocusRefetchPolicy = {
  staleTime: CHANNEL_TEMPLATES_FOCUS_STALE_TIME_MS,
  refetchOnWindowFocus: true,
} as const;

export const channelTemplatesQueryKey = ["channel-templates"] as const;

export function useChannelTemplatesQuery() {
  const refetchInterval = useFocusedRefetchInterval(
    CHANNEL_TEMPLATES_REFETCH_INTERVAL_MS,
  );

  return useQuery({
    queryKey: channelTemplatesQueryKey,
    queryFn: listChannelTemplates,
    refetchInterval,
    ...channelTemplatesFocusRefetchPolicy,
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

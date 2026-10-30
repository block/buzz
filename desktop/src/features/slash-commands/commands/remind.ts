import * as React from "react";

import { useReminderMutations } from "@/features/reminders/hooks";
import {
  formatRemindDue,
  parseRemindCommand,
  REMIND_PARSE_ERROR_MESSAGES,
} from "@/features/reminders/lib/remindTimeParser";
import { useIdentityQuery } from "@/shared/api/hooks";
import { SlashUsageError } from "../lib/slashUsageError";
import type {
  SlashCommandDefinition,
  SlashCommandEnvironment,
} from "../lib/types";

export function useRemindCommands(
  _environment: SlashCommandEnvironment,
): readonly SlashCommandDefinition[] {
  const pubkey = useIdentityQuery().data?.pubkey;
  const { mutateAsync: createNote } = useReminderMutations(
    pubkey ?? "",
  ).createNote;

  return React.useMemo(
    () => [
      {
        name: "remind",
        argumentHint: "me <what> <when>",
        description: "Set a reminder for yourself",
        usage: "/remind me <what> <when>",
        context: "any",
        run: async ({ args, toast }) => {
          const parsed = parseRemindCommand(args, { now: Date.now() });
          if (!parsed.ok) {
            throw new SlashUsageError(
              REMIND_PARSE_ERROR_MESSAGES[parsed.error] ?? undefined,
            );
          }
          if (!pubkey) throw new Error("Sign in to set reminders");
          await createNote({
            note: parsed.note,
            notBefore: Math.floor(parsed.dueAt / 1_000),
          });
          toast.success(`Reminder set for ${formatRemindDue(parsed.dueAt)}`);
        },
      },
    ],
    [createNote, pubkey],
  );
}

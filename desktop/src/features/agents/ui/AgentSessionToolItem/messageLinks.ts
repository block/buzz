import type { TranscriptItem } from "../agentSessionTypes";
import {
  asRecord,
  getToolString,
  parseToolResultValue,
} from "../agentSessionUtils";

export type SentMessageLink = {
  channelId: string;
  messageId: string;
};

export function getSentMessageLink(
  item: Extract<TranscriptItem, { type: "tool" }>,
): SentMessageLink | null {
  if (item.status !== "completed" || item.isError) {
    return null;
  }

  if (item.descriptor?.renderClass !== "message") {
    return null;
  }

  const channelId =
    item.channelId ?? getToolString(item.args, ["channel_id", "channelId"]);
  if (!channelId) {
    return null;
  }

  const resultRecord = getMessageSendResultRecord(item.result);
  if (!resultRecord || resultRecord.accepted === false) {
    return null;
  }

  const messageId = getToolString(resultRecord, [
    "event_id",
    "eventId",
    "message_id",
    "messageId",
  ]);
  if (!messageId) {
    return null;
  }

  return {
    channelId,
    messageId,
  };
}

function getMessageSendResultRecord(
  result: string,
): Record<string, unknown> | null {
  const parsed = parseToolResultValue(result);
  const directRecord = asRecord(parsed);
  if (getMessageEventId(directRecord)) {
    return directRecord;
  }

  const stdout = getToolString(directRecord, ["stdout"]);
  if (!stdout) {
    return null;
  }

  const stdoutRecord = asRecord(parseToolResultValue(stdout));
  return getMessageEventId(stdoutRecord) ? stdoutRecord : null;
}

function getMessageEventId(record: Record<string, unknown>) {
  return getToolString(record, [
    "event_id",
    "eventId",
    "message_id",
    "messageId",
  ]);
}

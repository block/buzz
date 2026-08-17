import { invokeTauri } from "@/shared/api/tauri";

type PublishEventResponse = {
  accepted: boolean;
  event_id: string;
  message: string;
};

type FlowGraphResponse = {
  flow_id: string;
  graph_json: string | null;
  found: boolean;
  event_id?: string;
};

export function publishFlowGraph(
  flowId: string,
  graphJson: string,
): Promise<PublishEventResponse> {
  return invokeTauri("publish_flow_graph", { flowId, graphJson });
}

export function getFlowGraph(flowId: string): Promise<FlowGraphResponse> {
  return invokeTauri("get_flow_graph", { flowId });
}

export function publishSkillImport(
  skillId: string,
  sourceRepo?: string | null,
  sourceCommit?: string | null,
): Promise<PublishEventResponse> {
  return invokeTauri("publish_skill_import", {
    skillId,
    sourceRepo: sourceRepo ?? null,
    sourceCommit: sourceCommit ?? null,
  });
}

export function publishKbDocument(input: {
  knowledgeBaseId: string;
  documentId: string;
  filename: string;
  mimeType: string;
  content?: string | null;
}): Promise<PublishEventResponse> {
  return invokeTauri("publish_kb_document", {
    knowledgeBaseId: input.knowledgeBaseId,
    documentId: input.documentId,
    filename: input.filename,
    mimeType: input.mimeType,
    content: input.content ?? null,
  });
}

export function publishTableRow(
  tableId: string,
  rowId: string,
  rowJson: string,
): Promise<PublishEventResponse> {
  return invokeTauri("publish_table_row", { tableId, rowId, rowJson });
}

export function deleteTableRow(
  tableId: string,
  rowId: string,
): Promise<PublishEventResponse> {
  return invokeTauri("delete_table_row", { tableId, rowId });
}

export function publishFlowFile(
  fileId: string,
  filename: string,
  mediaUrl?: string | null,
): Promise<PublishEventResponse> {
  return invokeTauri("publish_flow_file", {
    fileId,
    filename,
    mediaUrl: mediaUrl ?? null,
  });
}

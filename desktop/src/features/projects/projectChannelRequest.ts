export const PROJECT_CHANNEL_REQUEST = "project_channel_request" as const;

export type ProjectChannelRequest = {
  type: typeof PROJECT_CHANNEL_REQUEST;
  action: "create";
  requestId: string;
  request: {
    homeChannelId: string;
    name: string;
    description?: string;
    visibility: "open" | "private";
    ttlSeconds?: number;
    templateName?: string;
  };
};

function isText(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isTextWithin(value: unknown, max: number): value is string {
  return isText(value) && value.length <= max;
}

/** Parses the narrow, no-secret owner-review contract for project channels. */
export function parseProjectChannelRequest(
  value: unknown,
): ProjectChannelRequest | null {
  if (typeof value !== "object" || value === null) return null;
  const payload = value as Record<string, unknown>;
  if (
    payload.type !== PROJECT_CHANNEL_REQUEST ||
    payload.action !== "create" ||
    !isText(payload.requestId) ||
    typeof payload.request !== "object" ||
    payload.request === null
  ) {
    return null;
  }
  const request = payload.request as Record<string, unknown>;
  const allowed = [
    "homeChannelId",
    "name",
    "description",
    "visibility",
    "ttlSeconds",
    "templateName",
  ];
  if (
    Object.keys(request).some((key) => !allowed.includes(key)) ||
    !isTextWithin(request.homeChannelId, 128) ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(
      request.homeChannelId,
    ) ||
    !isTextWithin(request.name, 120) ||
    (request.visibility !== "open" && request.visibility !== "private") ||
    (request.description !== undefined &&
      !isTextWithin(request.description, 2_048)) ||
    (request.templateName !== undefined &&
      !isTextWithin(request.templateName, 300)) ||
    (request.ttlSeconds !== undefined &&
      (typeof request.ttlSeconds !== "number" ||
        !Number.isSafeInteger(request.ttlSeconds) ||
        request.ttlSeconds <= 0))
  ) {
    return null;
  }
  return {
    type: PROJECT_CHANNEL_REQUEST,
    action: "create",
    requestId: payload.requestId,
    request: {
      homeChannelId: request.homeChannelId,
      name: request.name,
      visibility: request.visibility,
      ...(request.description ? { description: request.description } : {}),
      ...(request.ttlSeconds ? { ttlSeconds: request.ttlSeconds } : {}),
      ...(request.templateName ? { templateName: request.templateName } : {}),
    },
  };
}

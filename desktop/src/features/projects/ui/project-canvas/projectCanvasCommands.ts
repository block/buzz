import { invokeTauri } from "@/shared/api/tauri";

import {
  parseProjectCanvasPackageDescriptor,
  parseProjectCanvasPackageDescriptorForE2e,
  parseProjectCanvasPendingUpdates,
  type ProjectCanvasPackageDescriptor,
  type ProjectCanvasPendingUpdates,
} from "./projectCanvasProtocol";

/** The Tauri command surface shared by the Canvas host and its frame. */

export type ProjectCanvasPackageRequest = {
  communityId: string;
  projectId: string;
};

const parsePackageDescriptor =
  import.meta.env.MODE === "e2e"
    ? parseProjectCanvasPackageDescriptorForE2e
    : parseProjectCanvasPackageDescriptor;

export async function requestProjectCanvasPackage(
  command: "activate_project_canvas_package" | "get_project_canvas_package",
  request: ProjectCanvasPackageRequest,
): Promise<ProjectCanvasPackageDescriptor> {
  const response = await invokeTauri<unknown>(command, { request });
  return parsePackageDescriptor(response);
}

/** One avatar's bytes, addressed by the pubkey its frame will request. */
export type ProjectCanvasAvatarUpload = {
  contentType: string;
  /** Standard base64 of the image bytes, without the data-URL prefix. */
  data: string;
  pubkey: string;
};

/**
 * Publishes avatar bytes for a project's canvas frames to serve from
 * `__buzz/avatar/<pubkey>`.
 *
 * Frames cannot reach the network, so this is how a real picture gets to a
 * widget without being base64'd into an RPC message and charged against its
 * 64 KiB ceiling. The backend keys the bytes by project rather than by load,
 * so publishing before or after a frame exists both work.
 */
export async function publishProjectCanvasAvatars(
  request: ProjectCanvasPackageRequest,
  avatars: ProjectCanvasAvatarUpload[],
): Promise<void> {
  if (avatars.length === 0) return;
  await invokeTauri("publish_project_canvas_avatars", { avatars, request });
}

export async function releaseProjectCanvasPackage(
  loadId: string,
): Promise<void> {
  await invokeTauri("release_project_canvas_package", { loadId });
}

export async function commitProjectCanvasPackage(
  loadId: string,
): Promise<void> {
  await invokeTauri("commit_project_canvas_package", { loadId });
}

export async function requestProjectCanvasUpdates(
  request: ProjectCanvasPackageRequest,
): Promise<ProjectCanvasPendingUpdates> {
  const response = await invokeTauri<unknown>("get_project_canvas_updates", {
    request,
  });
  return parseProjectCanvasPendingUpdates(response);
}

export async function openProjectCanvasSource(
  request: ProjectCanvasPackageRequest,
): Promise<void> {
  await invokeTauri("open_project_canvas_source", { request });
}

export function projectCanvasErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Canvas package failed.";
}

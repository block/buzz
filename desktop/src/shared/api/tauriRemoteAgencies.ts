import type {
  RemoteAgencyBinding,
  RemoteAgencyDescriptor,
} from "@/shared/api/remoteAgencyTypes";
import { invokeTauri } from "@/shared/api/tauri";

export async function previewRemoteAgency(
  sourceUrl: string,
): Promise<RemoteAgencyDescriptor> {
  return invokeTauri<RemoteAgencyDescriptor>("preview_remote_agency", {
    sourceUrl,
  });
}

export async function listRemoteAgencies(): Promise<RemoteAgencyBinding[]> {
  return invokeTauri<RemoteAgencyBinding[]>("list_remote_agencies");
}

export async function storeRemoteAgencyBearerToken(input: {
  recordUrl: string;
  endpoint: string;
  token: string;
}): Promise<void> {
  return invokeTauri<void>("store_remote_agency_bearer_token", input);
}

export async function saveRemoteAgencyBinding(
  binding: RemoteAgencyBinding,
): Promise<RemoteAgencyBinding> {
  return invokeTauri<RemoteAgencyBinding>("save_remote_agency_binding", {
    binding,
  });
}

import type {
  HuggingFaceModelFile,
  HuggingFaceModelSummary,
} from "@/shared/api/tauriMesh";

/**
 * Build the exact GGUF reference accepted by the pinned MeshLLM resolver.
 *
 * `hf://` deliberately is not used here: MeshLLM reserves that prefix for
 * layered package repositories. Raw model files use owner/repo@sha/file.gguf.
 */
export function immutableHuggingFaceModelRef(
  model: Pick<HuggingFaceModelSummary, "repoId" | "revision">,
  file: Pick<HuggingFaceModelFile, "path">,
): string {
  if (!/^[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/.test(model.repoId)) {
    throw new Error("Invalid Hugging Face repository id");
  }
  if (!/^(?:[a-fA-F0-9]{40}|[a-fA-F0-9]{64})$/.test(model.revision)) {
    throw new Error("Hugging Face model is missing an immutable revision");
  }
  if (
    file.path.length === 0 ||
    file.path.startsWith("/") ||
    file.path
      .split("/")
      .some((segment) => segment === "" || segment === "..") ||
    !file.path.toLowerCase().endsWith(".gguf")
  ) {
    throw new Error("Invalid Hugging Face GGUF path");
  }
  return `${model.repoId}@${model.revision}/${file.path}`;
}

export function formatModelBytes(sizeBytes: number | null): string | null {
  if (sizeBytes == null || !Number.isFinite(sizeBytes) || sizeBytes <= 0) {
    return null;
  }
  const gib = sizeBytes / 1024 ** 3;
  if (gib >= 1) return `${gib.toFixed(gib >= 10 ? 0 : 1)} GB`;
  const mib = sizeBytes / 1024 ** 2;
  return `${mib.toFixed(mib >= 10 ? 0 : 1)} MB`;
}

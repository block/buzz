export type BlobDescriptor = {
  url: string;
  sha256: string;
  size: number;
  type: string;
  uploaded: number;
  dim?: string;
  blurhash?: string;
  thumb?: string;
  duration?: number;
  image?: string;
  /** Original filename captured client-side. */
  filename?: string;
};

/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_BUZZ_DEEP_LINK_SCHEME?: string;
  readonly VITE_BUZZ_RELEASE_REPOSITORY?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_BUZZ_FEEDBACK_CHANNEL_ID?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

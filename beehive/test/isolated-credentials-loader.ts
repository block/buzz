import { registerHooks } from 'node:module';
import { fileURLToPath } from 'node:url';
const backend = fileURLToPath(new URL('./isolated-file-credentials.ts', import.meta.url));
// Explicit test-only module injection, never a CLI flag or production env mode.
registerHooks({ load(url, context, next) {
  const result = next(url, context);
  if (!url.endsWith('/src/credential-store.ts')) return result;
  return { ...result, source: String(result.source).replace(/export const systemCredentials: CredentialBackend = \{[\s\S]*?\n\};/, `import { isolatedFileCredentials } from ${JSON.stringify(backend)};\nexport const systemCredentials: CredentialBackend = isolatedFileCredentials(process.env.BEEHIVE_TEST_CREDENTIAL_FILE!);`) };
} });

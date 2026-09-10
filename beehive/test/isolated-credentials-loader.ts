import { registerHooks } from 'node:module';
import { fileURLToPath } from 'node:url';
const backend = fileURLToPath(new URL('./isolated-file-credentials.ts', import.meta.url));
// Explicit test-only module injection, never a CLI flag or production env mode.
registerHooks({ load(url, context, next) {
  const result = next(url, context);
  if (!url.endsWith('/src/credential-store.ts')) return result;
  if (!String(result.source).includes('export const systemCredentials: CredentialBackend = nativeCredentials();')) throw Error('Credential fixture injection failed closed');
  return { ...result, source: String(result.source).replace(/export const systemCredentials: CredentialBackend = nativeCredentials\(\);/, `import { isolatedFileCredentials } from ${JSON.stringify(backend)};\nexport const systemCredentials: CredentialBackend = isolatedFileCredentials(process.env.BEEHIVE_TEST_CREDENTIAL_FILE!);`) };
} });

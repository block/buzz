import { CheckCircle2 } from "lucide-react";

type Props = {
  signerKeyId: string | null;
  publicKeyFingerprint: string | null;
  privateKeyStorage: string | null;
  trustStorePath: string | null;
};

export function NxtlinqInitializationSuccess({
  signerKeyId,
  publicKeyFingerprint,
  privateKeyStorage,
  trustStorePath,
}: Props) {
  return (
    <section className="flex items-start gap-2 rounded-xl bg-emerald-500/10 p-4 text-sm text-emerald-700 dark:text-emerald-400">
      <CheckCircle2 className="mt-0.5 size-4 shrink-0" />
      <div>
        <p>
          Nxtlinq Attest initialized with a new owner-controlled signing key
          {signerKeyId ? ` (${signerKeyId})` : ""}.
        </p>
        {privateKeyStorage ? (
          <p className="mt-1 text-xs">
            Private key protected by {privateKeyStorage}. Buzz will retrieve it
            only in native code when you approve signing.
          </p>
        ) : null}
        {publicKeyFingerprint ? (
          <p className="mt-1 font-mono text-xs">{publicKeyFingerprint}</p>
        ) : null}
        {trustStorePath ? (
          <p className="mt-1 text-xs">
            Public signer enrolled in Buzz-managed trust: {trustStorePath}
          </p>
        ) : null}
      </div>
    </section>
  );
}

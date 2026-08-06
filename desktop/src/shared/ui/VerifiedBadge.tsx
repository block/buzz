import { useVerifiedIdentityExpiryRevision } from "@/shared/hooks/useVerifiedIdentityExpiry";
import { getCurrentVerifiedName } from "@/shared/lib/verifiedIdentity";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export function VerifiedBadge({
  verifiedName,
  verifiedNameExpiresAt,
}: {
  verifiedName: string;
  verifiedNameExpiresAt: number | null | undefined;
}) {
  useVerifiedIdentityExpiryRevision([verifiedNameExpiresAt]);
  const currentVerifiedName = getCurrentVerifiedName(
    verifiedName,
    verifiedNameExpiresAt,
  );
  if (!currentVerifiedName) {
    return null;
  }

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          aria-label={`Relay-verified identity: ${currentVerifiedName}`}
          className="inline-flex shrink-0 items-center"
          data-testid="relay-verified-identity"
          role="img"
        >
          <svg
            aria-hidden="true"
            className="h-4 w-4"
            fill="none"
            viewBox="0 0 24 24"
          >
            <path
              className="fill-blue-500"
              d="M3.85 8.62a4 4 0 0 1 4.78-4.77 4 4 0 0 1 6.74 0 4 4 0 0 1 4.78 4.78 4 4 0 0 1 0 6.74 4 4 0 0 1-4.77 4.78 4 4 0 0 1-6.75 0 4 4 0 0 1-4.78-4.77 4 4 0 0 1 0-6.76Z"
            />
            <path
              d="m8.75 12 2.1 2.1 4.4-4.4"
              stroke="white"
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth="2.4"
            />
          </svg>
        </span>
      </TooltipTrigger>
      <TooltipContent side="top">
        <p className="text-xs">Verified as {currentVerifiedName}</p>
      </TooltipContent>
    </Tooltip>
  );
}

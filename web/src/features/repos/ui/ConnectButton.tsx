import { ExternalLink } from "lucide-react";

import { relayWsUrl } from "@/shared/lib/relay-url";
import { Button } from "@/shared/ui/button";

export function ConnectButton({ className }: { className?: string }) {
  const deepLink = `buzz://connect?relay=${encodeURIComponent(relayWsUrl())}`;

  return (
    <Button asChild className={className}>
      <a href={deepLink}>
        <ExternalLink className="h-4 w-4" />
        Open in Buzz
      </a>
    </Button>
  );
}

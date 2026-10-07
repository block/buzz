import { invokeTauri } from "./tauri";

export type AcpCommandCandidate = {
  command: string;
  binaryPath: string;
};

export function discoverAcpCommands(): Promise<AcpCommandCandidate[]> {
  return invokeTauri<AcpCommandCandidate[]>("discover_acp_commands");
}

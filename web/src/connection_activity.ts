import type { AccountConnection, GatewayRepository } from "./provider";

export const CONNECTION_ACTIVITY_LIMIT = 20;

export async function connectionActivity(
  owner: Pick<GatewayRepository, "connectionProgress">,
): Promise<{ rows: AccountConnection[]; more: boolean }> {
  const rows = await owner.connectionProgress(CONNECTION_ACTIVITY_LIMIT + 1);
  return {
    rows: rows.slice(0, CONNECTION_ACTIVITY_LIMIT),
    more: rows.length > CONNECTION_ACTIVITY_LIMIT,
  };
}

export function connectionStatus(attempt: AccountConnection): string {
  return attempt.state === "failed"
    ? "Connection failed. Your previous connection has not been replaced."
    : "Connection not yet confirmed. A running check may still finish; after reopening, re-enter your password to retry.";
}

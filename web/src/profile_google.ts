// Server-mediated Google provider consent and the bounded Drive proxy. The
// browser only ever holds the session cookie; tokens stay on the beta server.
import type { Session } from "./auth";

export type CalendarChoice = "off" | "read" | "edit";
export interface RequestedServices {
  drive: boolean;
  calendar: CalendarChoice;
}
export interface GrantedAccess {
  drive: boolean;
  calendar_read: boolean;
  calendar_write: boolean;
}
export interface GoogleConnection {
  available: boolean;
  live: boolean;
  namespace: string | null;
  reason: string | null;
  connected: boolean;
  email: string;
  principal: string | null;
  requested: RequestedServices;
  granted: GrantedAccess;
  pending: RequestedServices | null;
}
export type DriveOperation =
  | { op: "about" }
  | { op: "list"; page_token?: string }
  | { op: "start_page_token" }
  | { op: "changes"; page_token: string }
  | { op: "metadata"; file_id: string }
  | { op: "media"; file_id: string }
  | { op: "generate_ids"; count: number }
  | { op: "create"; metadata: Record<string, unknown>; media: string };

export interface ProfileGoogleApi {
  connection(): Promise<GoogleConnection>;
  connect(request: RequestedServices): Promise<string>;
  disconnect(): Promise<void>;
  drive(operation: DriveOperation): Promise<unknown>;
}

export class GatewayProfileGoogle implements ProfileGoogleApi {
  constructor(private session: Session) {}
  private async json(path: string, body?: unknown): Promise<unknown> {
    const response = await fetch(path, {
      method: body === undefined ? "GET" : "POST",
      credentials: "same-origin",
      cache: "no-store",
      redirect: "error",
      headers:
        body === undefined
          ? { "x-shep-csrf": this.session.csrf }
          : {
              "x-shep-csrf": this.session.csrf,
              "content-type": "application/json",
            },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (response.status === 204) return null;
    if (response.status === 401)
      throw new Error("The beta session ended. Sign in again.");
    let value: unknown = null;
    try {
      value = await response.json();
    } catch {
      value = null;
    }
    if (!response.ok) {
      const message =
        typeof value === "object" &&
        value !== null &&
        "error" in value &&
        typeof value.error === "string"
          ? value.error
          : "The beta service could not complete this Google request. Retry.";
      throw new Error(message);
    }
    return value;
  }
  async connection(): Promise<GoogleConnection> {
    const value = await this.json("/api/profiles/connection");
    if (
      typeof value !== "object" ||
      value === null ||
      typeof (value as GoogleConnection).connected !== "boolean" ||
      typeof (value as GoogleConnection).available !== "boolean"
    )
      throw new Error("The beta service returned an invalid connection state.");
    return value as GoogleConnection;
  }
  async connect(request: RequestedServices): Promise<string> {
    const value = await this.json("/api/profiles/connect", request);
    const url =
      typeof value === "object" && value !== null && "url" in value
        ? value.url
        : null;
    if (
      typeof url !== "string" ||
      !url.startsWith("https://accounts.google.com/")
    )
      throw new Error("The beta service returned an invalid consent address.");
    return url;
  }
  async disconnect(): Promise<void> {
    await this.json("/api/profiles/disconnect", {});
  }
  drive(operation: DriveOperation): Promise<unknown> {
    return this.json("/api/profiles/drive", operation);
  }
}
/// Summarise saved and requested access the way Preferences shows it.
export function describeAccess(connection: GoogleConnection): string {
  if (!connection.connected) return "Not connected";
  const parts = [
    connection.granted.drive ? "Drive app data" : null,
    connection.granted.calendar_write
      ? "Calendar (edit)"
      : connection.granted.calendar_read
        ? "Calendar (read)"
        : null,
  ].filter(Boolean);
  return parts.length ? parts.join(" and ") : "No services granted";
}
export function describeRequest(requested: RequestedServices): string {
  const parts = [
    requested.drive ? "Drive app data" : null,
    requested.calendar === "edit"
      ? "Calendar (edit)"
      : requested.calendar === "read"
        ? "Calendar (read)"
        : null,
  ].filter(Boolean);
  return parts.length ? parts.join(" and ") : "No services";
}

// Node-side Google fixture for the browser profile scenarios: the beta API
// routes are answered from this process, and consent redirects come back to
// the app the way the backend's callback would. No network, no real account.
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import type { Page, Route } from "@playwright/test";

export const profile = "P".repeat(43);
export const NAMESPACE = "so.shep.fixture";
const sha256 = (text: string) =>
  createHash("sha256").update(text).digest("hex");
const namespaceDigest = sha256(NAMESPACE);
const golden = JSON.parse(
  readFileSync(
    new URL("../../shared/profile-operation.json", import.meta.url),
    "utf8",
  ),
);
export const uuid = (n: number, prefix = "7") =>
  `${prefix}0000000-0000-4000-8000-${n.toString(16).padStart(12, "0")}`;
function operation(
  profileId: string,
  n: number,
  parents: string[],
  changes: unknown[],
) {
  return {
    format: "so.shep.profile-operation",
    major: 1,
    minor: 0,
    requires: ["causal-v1", "accounts-v1", "settings-v1", "initialization-v1"],
    namespace: NAMESPACE,
    profile: profileId,
    generation: uuid(1, "9"),
    device: uuid(1, "d"),
    operation: uuid(n),
    parents,
    changes,
  };
}
export function publishedProfile(
  profileId: string,
  base: number,
  name: string,
  email = "alex@example.test",
) {
  const account = { ...golden.changes[0].account, id: uuid(base, "5"), email };
  const root = operation(
    profileId,
    base,
    [],
    [{ kind: "profile_setup", complete: false }],
  );
  const content = operation(
    profileId,
    base + 1,
    [root.operation],
    [
      { kind: "profile_name", name },
      { kind: "account_connection", account },
      { kind: "account_name", id: account.id, name: `${name} mailbox` },
      { kind: "setting", key: "appearance", value: "Dark" },
      { kind: "setting", key: "preview_lines", value: 3 },
      { kind: "setting", key: "tooltips", value: false },
    ],
  );
  const done = operation(
    profileId,
    base + 2,
    [content.operation],
    [{ kind: "profile_setup", complete: true }],
  );
  return [root, content, done];
}
interface File {
  id: string;
  name: string;
  mimeType: string;
  size: string;
  trashed: boolean;
  ownedByMe: boolean;
  spaces: string[];
  appProperties: Record<string, string>;
  media: string;
}
export class DriveFixture {
  files = new Map<string, File>();
  incompleteOnce = false;
  loseCreateReply = false;
  holdCreate: Promise<void> | null = null;
  calls: string[] = [];
  generated = 0;
  connected = false;
  granted = { drive: false, calendar_read: false, calendar_write: false };
  requested = { drive: true, calendar: "off" };
  consentOutcome: "connected" | "denied" | "failed" = "connected";
  connects: unknown[] = [];
  disconnects = 0;
  add(op: ReturnType<typeof operation>) {
    const media = JSON.stringify(op);
    const file: File = {
      id: `file-${op.operation.slice(-4)}`,
      name: `shep-profile-${op.operation}.json`,
      mimeType: "application/json",
      size: String(Buffer.byteLength(media)),
      trashed: false,
      ownedByMe: true,
      spaces: ["appDataFolder"],
      appProperties: {
        shepType: "profile",
        shepFormat: "operation-v1",
        shepNamespace: namespaceDigest,
        shepProfile: op.profile,
        shepGeneration: op.generation,
        shepOperation: op.operation,
        shepSha256: sha256(media),
      },
      media,
    };
    this.files.set(file.id, file);
  }
  private strip(file: File) {
    const { media: _, ...rest } = file;
    return rest;
  }
  connection() {
    return {
      available: true,
      live: false,
      namespace: NAMESPACE,
      reason: null,
      connected: this.connected,
      email: "owner@example.test",
      principal: this.connected ? "drive:fixture-permission" : null,
      requested: this.requested,
      granted: this.granted,
      pending: null,
    };
  }
  async drive(body: Record<string, unknown>): Promise<unknown> {
    this.calls.push(body.op as string);
    switch (body.op) {
      case "start_page_token":
        return { startPageToken: "1" };
      case "list": {
        const ids = [...this.files.keys()];
        const start = body.page_token ? Number(body.page_token) : 0;
        const slice = ids.slice(start, start + 50);
        const page: Record<string, unknown> = {
          incompleteSearch: this.incompleteOnce,
          files: slice.map((id) => this.strip(this.files.get(id)!)),
        };
        this.incompleteOnce = false;
        if (start + 50 < ids.length) page.nextPageToken = String(start + 50);
        return page;
      }
      case "changes":
        return { newStartPageToken: "1", changes: [] };
      case "metadata": {
        const file = this.files.get(body.file_id as string);
        return file ? this.strip(file) : { missing: true };
      }
      case "media":
        return { media: this.files.get(body.file_id as string)?.media ?? "" };
      case "generate_ids":
        return { ids: [`generated-${++this.generated}`] };
      case "create": {
        if (this.holdCreate) await this.holdCreate;
        const metadata = body.metadata as Record<string, unknown>;
        const media = body.media as string;
        const file: File = {
          id: metadata.id as string,
          name: metadata.name as string,
          mimeType: metadata.mimeType as string,
          size: String(Buffer.byteLength(media)),
          trashed: false,
          ownedByMe: true,
          spaces: ["appDataFolder"],
          appProperties: metadata.appProperties as Record<string, string>,
          media,
        };
        this.files.set(file.id, file);
        if (this.loseCreateReply) {
          this.loseCreateReply = false;
          throw new Error("lost");
        }
        return this.strip(file);
      }
    }
    throw new Error(`unexpected ${body.op}`);
  }
  /// Install every route the profile UI uses.
  async install(page: Page, origin = "http://127.0.0.1:5180") {
    await page.route("**/api/session", (r) =>
      r.fulfill({
        json: {
          email: "owner@example.test",
          user_id: profile,
          csrf: "X".repeat(43),
        },
      }),
    );
    await page.route("**/api/capabilities", (r) =>
      r.fulfill({
        json: {
          mail: true,
          endpoints: [
            { host: "mail.example.test", port: 993, service: "imap" },
            { host: "mail.example.test", port: 465, service: "smtp" },
          ],
          sent_copy: true,
        },
      }),
    );
    await page.route("**/api/mail/probe", (r) => r.fulfill({ json: {} }));
    await page.route("**/api/profiles/connection", (r) =>
      r.fulfill({ json: this.connection() }),
    );
    await page.route("**/api/profiles/connect", (r) => {
      this.connects.push(r.request().postDataJSON());
      this.requested = r.request().postDataJSON();
      return r.fulfill({
        json: {
          url: "https://accounts.google.com/o/oauth2/v2/auth?state=fixture",
        },
      });
    });
    await page.route("**/api/profiles/disconnect", (r) => {
      this.disconnects++;
      this.connected = false;
      this.granted = {
        drive: false,
        calendar_read: false,
        calendar_write: false,
      };
      return r.fulfill({ status: 204 });
    });
    await page.route("**/api/profiles/drive", async (r: Route) => {
      try {
        r.fulfill({ json: await this.drive(r.request().postDataJSON()) });
      } catch {
        r.fulfill({
          status: 502,
          json: { error: "Google Drive did not answer. Retry discovery." },
        });
      }
    });
    // The consent screen never loads; the "callback" lands back in the app.
    await page.route("https://accounts.google.com/**", (r) => {
      if (this.consentOutcome === "connected") {
        this.connected = true;
        this.granted = {
          drive: this.requested.drive,
          calendar_read: this.requested.calendar !== "off",
          calendar_write: this.requested.calendar === "edit",
        };
      }
      return r.fulfill({
        status: 302,
        headers: { location: `${origin}/#profiles=${this.consentOutcome}` },
      });
    });
  }
}
export async function seedAccounts(page: Page, emails: string[]) {
  await page.route("**/seed-profiles", (r) =>
    r.fulfill({
      contentType: "text/html",
      body: "<!doctype html><title>Profile fixture setup</title>",
    }),
  );
  await page.goto("/seed-profiles");
  await page.evaluate(
    async ([profile, emails]) => {
      const path = "/src/storage.ts";
      const { BrowserStore } = await import(path);
      const store = await BrowserStore.open(profile);
      await store.commit(
        (emails as string[]).map((email, i) => ({
          store: "accounts",
          key: `seed-${i}`,
          value: {
            id: `seed-${i}`,
            name: email,
            email,
            protocol: "Imap",
            host: "mail.example.test",
            port: 993,
            username: email,
            incoming_security: "Tls",
            incoming_auth: "Password",
            smtp_host: "mail.example.test",
            smtp_port: 465,
            smtp_username: email,
            smtp_security: "Tls",
            smtp_auth: "Automatic",
            smtp_separate_password: false,
            sent_copy: "Automatic",
            sent_folder: "",
          },
        })),
      );
      store.close();
    },
    [profile, emails] as const,
  );
}

/** Fixture Google provider and synthetic profile records for the browser
 * profile tests. Everything stays in memory; no network, no real account. */
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import * as wasm from "../wasm/shep_profile_core";
import type {
  DriveOperation,
  GoogleConnection,
  ProfileGoogleApi,
  RequestedServices,
} from "../profile_google";
import type { HistoryModule } from "../profile_history";
import type { Operation, Change } from "../profile_types";

export const NAMESPACE = "so.shep.fixture";
export const PRINCIPAL = "drive:fixture-permission";
export const IDENTITY = "P".repeat(43);
export const sha256 = (text: string) =>
  createHash("sha256").update(text).digest("hex");
export const namespaceDigest = sha256(NAMESPACE);
let initialised = false;
export function wasmModule(): HistoryModule {
  if (!initialised) {
    wasm.initSync({
      module: new WebAssembly.Module(
        readFileSync(
          new URL("../wasm/shep_profile_core_bg.wasm", import.meta.url),
        ),
      ),
    });
    initialised = true;
  }
  return wasm;
}
export const golden: Operation = JSON.parse(
  readFileSync(
    new URL("../../../shared/profile-operation.json", import.meta.url),
    "utf8",
  ),
);
export const fixtureAccount = golden.changes.find(
  (c) => c.kind === "account_connection",
) as Extract<Change, { kind: "account_connection" }>;
export function uuid(n: number, prefix = "7") {
  return `${prefix}0000000-0000-4000-8000-${n.toString(16).padStart(12, "0")}`;
}
export function operation(
  profile: string,
  n: number,
  parents: string[],
  changes: Change[],
  device = uuid(1, "d"),
): Operation {
  return {
    format: "so.shep.profile-operation",
    major: 1,
    minor: 0,
    requires: ["causal-v1", "accounts-v1", "settings-v1", "initialization-v1"],
    namespace: NAMESPACE,
    profile,
    generation: uuid(1, "9"),
    device,
    operation: uuid(n),
    parents,
    changes,
  };
}
/** A complete initialised profile: preparing root, content, completion. */
export function publishedProfile(profile: string, base: number, name: string) {
  const root = operation(
    profile,
    base,
    [],
    [{ kind: "profile_setup", complete: false }],
  );
  const content = operation(
    profile,
    base + 1,
    [root.operation],
    [
      { kind: "profile_name", name },
      {
        kind: "account_connection",
        account: { ...fixtureAccount.account, id: uuid(base, "5") },
      },
      { kind: "account_name", id: uuid(base, "5"), name: `${name} mailbox` },
      { kind: "setting", key: "appearance", value: "Dark" },
      { kind: "setting", key: "preview_lines", value: 3 },
      { kind: "setting", key: "tooltips", value: false },
    ],
  );
  const done = operation(
    profile,
    base + 2,
    [content.operation],
    [{ kind: "profile_setup", complete: true }],
  );
  return [root, content, done];
}
export interface FixtureFile {
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
export function fileFor(
  op: Operation,
  id = `file-${op.operation.slice(-4)}`,
): FixtureFile {
  const media = JSON.stringify(op);
  return {
    id,
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
}
export class FakeGoogleApi implements ProfileGoogleApi {
  files = new Map<string, FixtureFile>();
  changes: { fileId: string; removed: boolean }[] = [];
  changeToken = 1;
  pageSize = 50;
  incompleteOnce = false;
  failNext: Partial<Record<DriveOperation["op"], number>> = {};
  /** Throw after the fixture applied the create, as a lost reply would. */
  loseCreateReply = false;
  calls: DriveOperation["op"][] = [];
  generated = 0;
  connected: GoogleConnection = {
    available: true,
    live: false,
    namespace: NAMESPACE,
    reason: null,
    connected: true,
    email: "owner@example.test",
    principal: PRINCIPAL,
    requested: { drive: true, calendar: "off" },
    granted: { drive: true, calendar_read: false, calendar_write: false },
    pending: null,
  };
  connectUrls: RequestedServices[] = [];
  disconnects = 0;
  connection(): Promise<GoogleConnection> {
    return Promise.resolve(structuredClone(this.connected));
  }
  connect(request: RequestedServices): Promise<string> {
    this.connectUrls.push(request);
    return Promise.resolve(
      "https://accounts.google.com/o/oauth2/v2/auth?state=fixture",
    );
  }
  disconnect(): Promise<void> {
    this.disconnects++;
    this.connected = { ...this.connected, connected: false, principal: null };
    return Promise.resolve();
  }
  add(op: Operation, id?: string) {
    const file = fileFor(op, id);
    this.files.set(file.id, file);
    return file;
  }
  private strip(file: FixtureFile) {
    const { media: _, ...rest } = file;
    return rest;
  }
  async drive(operation: DriveOperation): Promise<unknown> {
    this.calls.push(operation.op);
    const remaining = this.failNext[operation.op];
    if (remaining) {
      this.failNext[operation.op] = remaining - 1;
      throw new Error(`Fixture Drive failed ${operation.op}.`);
    }
    switch (operation.op) {
      case "about":
        return { user: { permissionId: "fixture-permission" } };
      case "start_page_token":
        return { startPageToken: String(this.changeToken) };
      case "list": {
        const ids = [...this.files.keys()];
        const start = operation.page_token ? Number(operation.page_token) : 0;
        const slice = ids.slice(start, start + this.pageSize);
        const page: Record<string, unknown> = {
          incompleteSearch: this.incompleteOnce,
          files: slice.map((id) => this.strip(this.files.get(id)!)),
        };
        this.incompleteOnce = false;
        if (start + this.pageSize < ids.length)
          page.nextPageToken = String(start + this.pageSize);
        return page;
      }
      case "changes": {
        const since = Number(operation.page_token);
        const pending = this.changes.slice(since - this.changeToken);
        return {
          newStartPageToken: String(this.changeToken + this.changes.length),
          changes: pending.map((c) => ({
            fileId: c.fileId,
            removed: c.removed,
            file: c.removed ? undefined : this.strip(this.files.get(c.fileId)!),
          })),
        };
      }
      case "metadata": {
        const file = this.files.get(operation.file_id);
        return file ? this.strip(file) : { missing: true };
      }
      case "media": {
        const file = this.files.get(operation.file_id);
        if (!file) throw new Error("Fixture file missing.");
        return { media: file.media };
      }
      case "generate_ids":
        return {
          ids: Array.from(
            { length: operation.count },
            () => `generated-${++this.generated}`,
          ),
        };
      case "create": {
        const metadata = operation.metadata as Record<string, unknown>;
        const file: FixtureFile = {
          id: metadata.id as string,
          name: metadata.name as string,
          mimeType: metadata.mimeType as string,
          size: String(Buffer.byteLength(operation.media)),
          trashed: false,
          ownedByMe: true,
          spaces: ["appDataFolder"],
          appProperties: metadata.appProperties as Record<string, string>,
          media: operation.media,
        };
        this.files.set(file.id, file);
        if (this.loseCreateReply) {
          this.loseCreateReply = false;
          throw new Error("Fixture lost the upload reply.");
        }
        return this.strip(file);
      }
    }
  }
}

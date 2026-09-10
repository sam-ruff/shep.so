// Reviewed first-profile publication (docs/agents/PROFILE_PUBLICATION.md in
// browser form): a frozen review of account definitions and portable
// preferences, staged exact local edits, then one owned Drive upload per step
// with verified receipts. Pause finishes the accepted step; retry reuses the
// same operation identities, reserved file IDs and exact bytes.
import type { Account } from "./provider";
import type { ProfileGoogleApi } from "./profile_google";
import { ProfileJournal, type HistoryPort } from "./profile_history";
import {
  fileMetadata,
  isMissing,
  parseDriveFile,
  parseMedia,
  sha256Hex,
  verifyFileMetadata,
} from "./profile_drive";
import type { ProfileDiscovery } from "./profile_discovery";
import type {
  AccountMapping,
  CatalogFile,
  ProfileStore,
} from "./profile_store";
import {
  BROWSER_SETTINGS,
  type BrowserSettingKey,
  type PortableValues,
} from "./profile_settings";
import {
  MAX_CHANGES,
  bindingKey,
  isUuid,
  type Binding,
  type Change,
  type LocalEdit,
  type PortableConnection,
} from "./profile_types";

export interface PublicationAccountRow {
  position: number;
  local: string;
  shared: string;
  account: Account;
  selected: boolean;
}
export interface PublicationSettingRow {
  key: BrowserSettingKey;
  value: unknown;
  selected: boolean;
}
export interface PublicationReview {
  id: string;
  name: string;
  profile: string;
  generation: string;
  accounts: PublicationAccountRow[];
  settings: PublicationSettingRow[];
  fingerprint: string;
  phase: "review" | "staging" | "uploading" | "complete";
  edits: LocalEdit[];
  staged: number;
  uploaded: number;
  total: number;
  failed: boolean;
  error: string | null;
}
export function portableAccount(
  shared: string,
  a: Account,
): PortableConnection {
  return {
    id: shared,
    email: a.email,
    protocol: a.protocol,
    host: a.host,
    port: a.port,
    username: a.username,
    incoming_security: a.incoming_security,
    incoming_auth: a.incoming_auth,
    smtp_host: a.smtp_host,
    smtp_port: a.smtp_port,
    smtp_username: a.smtp_username || a.username,
    smtp_security: a.smtp_security,
    smtp_auth: a.smtp_auth,
    smtp_separate_password: a.smtp_separate_password,
    sent_copy: a.sent_copy,
    sent_folder: a.sent_folder,
  };
}
export async function fingerprintOf(
  accounts: Account[],
  values: PortableValues["values"],
): Promise<string> {
  const sorted = [...accounts].sort((a, b) => a.id.localeCompare(b.id));
  return sha256Hex(JSON.stringify([sorted, values]));
}
export class ProfilePublication extends EventTarget {
  review: PublicationReview | null = null;
  busy = false;
  paused = false;
  private generation = 0;
  private journal: ProfileJournal | null = null;
  constructor(
    readonly store: ProfileStore,
    private api: ProfileGoogleApi,
    private port: HistoryPort,
    private discovery: ProfileDiscovery,
  ) {
    super();
  }
  private changed() {
    this.dispatchEvent(new Event("change"));
  }
  async load() {
    this.review =
      (await this.store.get<PublicationReview>("publications", "current")) ??
      null;
    this.changed();
  }
  invalidate() {
    this.generation++;
    this.busy = false;
    void this.journal?.close();
    this.journal = null;
    this.changed();
  }
  private async save() {
    await this.store.commit([
      {
        store: "publications",
        key: "current",
        value: this.review ?? undefined,
      },
    ]);
  }
  /// Freeze the review: every account gets a durable shared UUID mapping and
  /// the current portable preferences are captured exactly.
  async prepare(name: string, accounts: Account[], captured: PortableValues) {
    if (this.review && this.review.phase !== "review")
      throw new Error(
        "A profile publication is already in progress. Resume or finish it first.",
      );
    if (!this.discovery.complete)
      throw new Error(
        "Finish discovery before creating a profile; a failed or incomplete listing is not an empty Google account.",
      );
    const trimmed = name.trim();
    if (!trimmed || trimmed.length > 256)
      throw new Error("Give the profile a name of up to 256 characters.");
    const rows: PublicationAccountRow[] = [];
    const mappings: {
      store: "mappings";
      key: string;
      value: AccountMapping;
    }[] = [];
    for (const [position, account] of accounts.entries()) {
      const existing = await this.store.get<AccountMapping>(
        "mappings",
        account.id,
      );
      const shared =
        existing?.shared ??
        (isUuid(account.id) ? account.id : crypto.randomUUID());
      if (!existing)
        mappings.push({
          store: "mappings",
          key: account.id,
          value: {
            local: account.id,
            shared,
            source: "publication",
            reconnect: false,
          },
        });
      rows.push({
        position,
        local: account.id,
        shared,
        account: structuredClone(account),
        selected: true,
      });
    }
    this.review = {
      id: crypto.randomUUID(),
      name: trimmed,
      profile: crypto.randomUUID(),
      generation: crypto.randomUUID(),
      accounts: rows,
      settings: BROWSER_SETTINGS.map((key) => ({
        key,
        value: captured.values[key],
        selected: true,
      })),
      fingerprint: await fingerprintOf(accounts, captured.values),
      phase: "review",
      edits: [],
      staged: 0,
      uploaded: 0,
      total: 0,
      failed: false,
      error: null,
    };
    await this.store.commit([
      ...mappings,
      { store: "publications", key: "current", value: this.review },
    ]);
    this.changed();
  }
  rows(after: number): PublicationAccountRow[] {
    return (this.review?.accounts ?? []).slice(after, after + 50);
  }
  async chooseAccount(position: number, selected: boolean) {
    if (!this.review || this.review.phase !== "review") return;
    const row = this.review.accounts[position];
    if (row) row.selected = selected;
    await this.save();
    this.changed();
  }
  async chooseSetting(key: BrowserSettingKey, selected: boolean) {
    if (!this.review || this.review.phase !== "review") return;
    const row = this.review.settings.find((s) => s.key === key);
    if (row) row.selected = selected;
    await this.save();
    this.changed();
  }
  async cancel() {
    if (!this.review || this.busy) return;
    if (this.review.phase === "uploading" && this.review.uploaded > 0)
      throw new Error(
        "Uploaded profile files are immutable. Finish the publication or leave it paused.",
      );
    this.review = null;
    await this.save();
    this.changed();
  }
  /// Approval rechecks the frozen values against the current setup and
  /// freezes every operation the publication will stage.
  async approve(accounts: Account[], captured: PortableValues) {
    const review = this.review;
    if (!review || review.phase !== "review" || this.busy) return;
    if (!this.discovery.complete)
      throw new Error(
        "Finish discovery before publishing; the Google account listing is incomplete.",
      );
    if ((await fingerprintOf(accounts, captured.values)) !== review.fingerprint)
      throw new Error(
        "Accounts or preferences changed since this review. Cancel it and prepare the review again.",
      );
    if (
      !review.accounts.some((r) => r.selected) &&
      !review.settings.some((s) => s.selected)
    )
      throw new Error("Choose at least one account or setting to publish.");
    const changes: Change[] = [{ kind: "profile_name", name: review.name }];
    for (const row of review.accounts.filter((r) => r.selected)) {
      changes.push({
        kind: "account_connection",
        account: portableAccount(row.shared, row.account),
      });
      if (row.account.name.trim())
        changes.push({
          kind: "account_name",
          id: row.shared,
          name: row.account.name.trim(),
        });
    }
    for (const setting of review.settings.filter((s) => s.selected))
      changes.push({ kind: "setting", key: setting.key, value: setting.value });
    const edits: LocalEdit[] = [
      {
        operation: crypto.randomUUID(),
        expected_revision: 0,
        changes: [{ kind: "profile_setup", complete: false }],
      },
    ];
    for (let i = 0; i < changes.length; i += MAX_CHANGES)
      edits.push({
        operation: crypto.randomUUID(),
        expected_revision: 0,
        changes: changes.slice(i, i + MAX_CHANGES),
      });
    edits.push({
      operation: crypto.randomUUID(),
      expected_revision: 0,
      changes: [{ kind: "profile_setup", complete: true }],
    });
    review.edits = edits;
    review.total = edits.length;
    review.phase = "staging";
    await this.save();
    await this.run();
  }
  pause() {
    this.paused = true;
    this.changed();
  }
  async resume() {
    if (!this.review || this.review.phase === "review") return;
    if (!this.discovery.complete)
      throw new Error("Finish discovery before continuing this publication.");
    this.paused = false;
    await this.run();
  }
  retry() {
    return this.resume();
  }
  private binding(): Binding {
    return {
      namespace: this.discovery.scope.namespace,
      principal: this.discovery.scope.principal,
      profile: this.review!.profile,
      generation: this.review!.generation,
    };
  }
  async localJournal(): Promise<ProfileJournal> {
    if (this.journal) return this.journal;
    const binding = this.binding();
    const key = `local:${bindingKey(binding)}`;
    this.journal = await ProfileJournal.open(
      this.port,
      key,
      binding,
      await this.store.deviceFor(key),
      await this.store.records(key),
    );
    return this.journal;
  }
  private async run() {
    if (this.busy || !this.review) return;
    this.busy = true;
    this.paused = false;
    const generation = this.generation;
    this.review.failed = false;
    this.review.error = null;
    try {
      await this.save();
      this.changed();
      while (
        this.generation === generation &&
        !this.paused &&
        this.review &&
        this.review.phase !== "complete" &&
        !this.review.failed
      )
        await this.step(generation);
    } catch (error) {
      if (this.generation !== generation || !this.review) return;
      this.review.failed = true;
      this.review.error =
        error instanceof Error
          ? error.message
          : "Publication failed. Retry the same step.";
      try {
        await this.save();
      } catch {
        // The failure is still visible in memory.
      }
    } finally {
      if (this.generation === generation) {
        this.busy = false;
        this.changed();
      }
    }
  }
  private fence(generation: number) {
    if (this.generation !== generation)
      throw new Error(
        "The Google connection changed; this step was discarded.",
      );
  }
  private async step(generation: number) {
    const review = this.review!;
    const journal = await this.localJournal();
    if (review.phase === "staging") {
      if (review.staged >= review.edits.length) {
        review.phase = "uploading";
        await this.save();
        return;
      }
      const edit = review.edits[review.staged];
      // The exact staged request is durable before the journal receives it;
      // a lost reply resubmits the identical request.
      const staged = (await this.store.stagedEdits(journal.key)).find(
        (e) => e.operation === edit.operation,
      );
      const request: LocalEdit = staged ?? {
        ...edit,
        expected_revision: (await journal.state()).revision,
      };
      if (!staged) await this.store.stageEdit(journal.key, request);
      await journal.execute({ kind: "edit", edit: request }, "state");
      this.fence(generation);
      await this.store.saveRecord(
        journal.key,
        await journal.record(edit.operation),
      );
      review.staged++;
      await this.save();
      this.changed();
      return;
    }
    if (review.phase === "uploading") {
      const upload = await journal.execute({ kind: "next_upload" }, "upload");
      if (!upload) {
        await this.discovery.summarise(journal);
        review.phase = "complete";
        await this.save();
        await this.discovery.firstPage();
        return;
      }
      let fileId = upload.file_id;
      if (!fileId) {
        const generated = await this.api.drive({
          op: "generate_ids",
          count: 1,
        });
        this.fence(generation);
        const ids =
          typeof generated === "object" &&
          generated !== null &&
          "ids" in generated
            ? (generated as { ids: unknown }).ids
            : null;
        fileId =
          Array.isArray(ids) && typeof ids[0] === "string" ? ids[0] : null;
        if (!fileId || !/^[A-Za-z0-9_-]{1,255}$/.test(fileId))
          throw new Error(
            "Google Drive did not reserve a file identity. Retry.",
          );
        await journal.execute(
          { kind: "reserve", operation: upload.operation, file_id: fileId },
          "state",
        );
        await this.store.saveRecord(
          journal.key,
          await journal.record(upload.operation),
        );
      }
      const namespaceDigest = await sha256Hex(this.discovery.scope.namespace);
      const operation = JSON.parse(upload.record);
      let metadata = await this.api.drive({ op: "metadata", file_id: fileId });
      this.fence(generation);
      if (isMissing(metadata)) {
        // Only a confirmed 404 permits the single create attempt.
        await this.api.drive({
          op: "create",
          metadata: fileMetadata(
            fileId,
            namespaceDigest,
            operation,
            upload.sha256,
          ),
          media: upload.record,
        });
        this.fence(generation);
        metadata = await this.api.drive({ op: "metadata", file_id: fileId });
        this.fence(generation);
      }
      const file = parseDriveFile(metadata);
      if (!file)
        throw new Error(
          "The uploaded profile file could not be read back. Retry the upload.",
        );
      const identity = verifyFileMetadata(file, namespaceDigest);
      if (
        identity.operation !== upload.operation ||
        identity.sha256 !== upload.sha256
      )
        throw new Error(
          "The reserved Google file holds a different record. Keep this publication paused and review the Google account.",
        );
      const media = parseMedia(
        await this.api.drive({ op: "media", file_id: fileId }),
      );
      this.fence(generation);
      if (media !== upload.record)
        throw new Error(
          "The uploaded profile file does not contain the exact record. Retry the upload.",
        );
      // Record the own-upload identity in discovery before confirming the queue.
      await this.store.commit([
        {
          store: "catalog",
          key: fileId,
          value: {
            id: fileId,
            name: file.name,
            operation: upload.operation,
            profile: identity.profile,
            generation: identity.generation,
            sha256: upload.sha256,
            size: file.size,
            origin: "published",
          } satisfies CatalogFile,
        },
      ]);
      await journal.execute(
        {
          kind: "confirm",
          operation: upload.operation,
          file_id: fileId,
          sha256: upload.sha256,
        },
        "state",
      );
      await this.store.saveRecord(
        journal.key,
        await journal.record(upload.operation),
      );
      review.uploaded++;
      await this.save();
      this.changed();
    }
  }
  dispose() {
    this.invalidate();
  }
}

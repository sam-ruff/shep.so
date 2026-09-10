// Reviewed browser enrollment (docs/agents/PROFILE_ENROLLMENT.md in browser
// form): copy originals from the observation journal into an independently
// owned local journal, review account definitions and portable preferences,
// then apply one account per step and the preferences through the device
// receipt. Mail, drafts and credentials are never touched; imported accounts
// require reviewed credential activation before any provider request.
import type { Account } from "./provider";
import { ProfileJournal, type HistoryPort } from "./profile_history";
import type { ProfileDiscovery } from "./profile_discovery";
import type {
  AccountMapping,
  ProfileStore,
  ProfileSummary,
} from "./profile_store";
import {
  BROWSER_SETTINGS,
  defaultPortable,
  type ApplyReceipt,
  type ApplyRequest,
  type BrowserSettingKey,
  type PortableValues,
} from "./profile_settings";
import {
  bindingKey,
  type Binding,
  type Change,
  type Field,
  type PortableConnection,
  type SettingKey,
} from "./profile_types";

export interface EnrollmentRow {
  position: number;
  target: string;
  kind: "account" | "name" | "setting";
  shared?: string;
  account?: PortableConnection;
  name?: string;
  local?: Account;
  key?: SettingKey;
  value?: unknown;
  reset?: boolean;
  available: boolean;
  reason?: string;
  selected: boolean;
}
export interface EnrollmentReview {
  id: string;
  profile: string;
  generation: string;
  name: string | null;
  sourceRevision: number;
  phase: "copying" | "review" | "applying" | "complete";
  cursor: number;
  copied: number;
  rows: EnrollmentRow[];
  includeAccounts: boolean;
  includeSettings: boolean;
  baseline: PortableValues;
  accountReceipts: Record<string, { local: string; committed: boolean }>;
  settingsRequest: ApplyRequest | null;
  settingsReceipt: ApplyReceipt | null;
  applied: number;
  kept: number;
  failed: boolean;
  error: string | null;
}
/// What enrollment needs from the mail side, kept behind an interface so the
/// controller is testable without the gateway.
export interface EnrollmentDevice {
  accounts(): Account[];
  importAccount(account: Account): Promise<void>;
  capture(): PortableValues;
  apply(request: ApplyRequest): ApplyReceipt;
  acknowledge(id: string): void;
}
export function localAccount(
  id: string,
  shared: PortableConnection,
  name: string | null,
  existing?: Account,
): Account {
  return {
    id,
    name: name ?? existing?.name ?? shared.email,
    email: shared.email,
    protocol: shared.protocol,
    host: shared.host,
    port: shared.port,
    username: shared.username,
    incoming_security: shared.incoming_security,
    incoming_auth: shared.incoming_auth,
    smtp_host: shared.smtp_host,
    smtp_port: shared.smtp_port,
    smtp_username: shared.smtp_username,
    smtp_security: shared.smtp_security,
    smtp_auth: shared.smtp_auth === "None" ? "Automatic" : shared.smtp_auth,
    smtp_separate_password: shared.smtp_separate_password,
    sent_copy: existing?.sent_copy ?? shared.sent_copy,
    sent_folder: existing?.sent_folder ?? shared.sent_folder,
  };
}
const connectionFields: (keyof Account)[] = [
  "email",
  "protocol",
  "host",
  "port",
  "username",
  "incoming_security",
  "incoming_auth",
  "smtp_host",
  "smtp_port",
  "smtp_username",
  "smtp_security",
  "smtp_auth",
  "smtp_separate_password",
];
function sameConnection(a: Account, b: PortableConnection): boolean {
  return connectionFields.every(
    (field) =>
      String(a[field]) === String(b[field as keyof PortableConnection]),
  );
}
export class ProfileEnrollment extends EventTarget {
  review: EnrollmentReview | null = null;
  busy = false;
  paused = false;
  private generation = 0;
  private journal: ProfileJournal | null = null;
  constructor(
    readonly store: ProfileStore,
    private port: HistoryPort,
    private discovery: ProfileDiscovery,
    private device: EnrollmentDevice,
  ) {
    super();
  }
  private changed() {
    this.dispatchEvent(new Event("change"));
  }
  async load() {
    this.review =
      (await this.store.get<EnrollmentReview>("enrollments", "current")) ??
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
  private save() {
    return this.store.commit([
      { store: "enrollments", key: "current", value: this.review ?? undefined },
    ]);
  }
  get needsReview() {
    return this.review?.phase === "review";
  }
  /// Prepare a review of one initialized profile after completed discovery.
  async prepare(summary: ProfileSummary) {
    if (
      this.review &&
      this.review.phase !== "review" &&
      this.review.phase !== "complete"
    )
      throw new Error(
        "An enrollment is already in progress. Resume or finish it first.",
      );
    if (!this.discovery.complete)
      throw new Error(
        "Finish discovery before importing accounts; an incomplete listing is not a complete profile.",
      );
    if (
      summary.removed ||
      !summary.initialized ||
      summary.waiting > 0 ||
      summary.ready > 0
    )
      throw new Error(
        "Finish discovery and profile setup before importing accounts. This profile is not complete.",
      );
    this.review = {
      id: crypto.randomUUID(),
      profile: summary.profile,
      generation: summary.generation,
      name: summary.name,
      sourceRevision: summary.revision,
      phase: "copying",
      cursor: 0,
      copied: 0,
      rows: [],
      includeAccounts: true,
      includeSettings: true,
      baseline: this.device.capture(),
      accountReceipts: {},
      settingsRequest: null,
      settingsReceipt: null,
      applied: 0,
      kept: 0,
      failed: false,
      error: null,
    };
    await this.save();
    await this.run();
  }
  rows(after: number): EnrollmentRow[] {
    return (this.review?.rows ?? []).slice(after, after + 50);
  }
  async choose(position: number, selected: boolean) {
    const row = this.review?.rows[position];
    if (!row || this.review?.phase !== "review" || !row.available) return;
    row.selected = selected;
    await this.save();
    this.changed();
  }
  async cancel() {
    if (!this.review || this.busy) return;
    if (this.review.phase === "applying")
      throw new Error(
        "Applied accounts are kept. Finish or leave this enrollment paused.",
      );
    this.review = null;
    await this.save();
    this.changed();
  }
  async approve(includeAccounts: boolean, includeSettings: boolean) {
    const review = this.review;
    if (!review || review.phase !== "review" || this.busy) return;
    review.includeAccounts = includeAccounts;
    review.includeSettings = includeSettings;
    review.phase = "applying";
    await this.save();
    await this.run();
  }
  pause() {
    this.paused = true;
    this.changed();
  }
  async resume() {
    if (
      !this.review ||
      this.review.phase === "review" ||
      this.review.phase === "complete"
    )
      return;
    if (this.review.phase === "copying" && !this.discovery.complete)
      throw new Error("Finish discovery before continuing this enrollment.");
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
  /// The independently owned local journal; the originating device reuses its
  /// publication journal rather than copying observation state.
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
        this.review.phase !== "review" &&
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
          : "Enrollment failed. Retry the same step.";
      try {
        await this.save();
      } catch {
        // Visible in memory regardless.
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
    if (review.phase === "copying") {
      const source = await this.discovery.journal(this.binding());
      const local = await this.localJournal();
      const next = await source.execute(
        {
          kind: "export_record",
          expected_revision: review.sourceRevision,
          after: review.cursor,
        },
        "record",
      );
      this.fence(generation);
      if (!next) {
        const state = await local.state();
        if (!state.initialized)
          throw new Error(
            "The copied profile history is incomplete. Retry discovery before importing it.",
          );
        review.rows = await this.buildRows(local);
        review.phase = "review";
        await this.save();
        return;
      }
      // Idempotent: a lost cursor save re-imports the same exact bytes.
      await this.discovery.importRecord(local, next.record);
      review.cursor = next.position;
      review.copied++;
      await this.save();
      this.changed();
      return;
    }
    if (review.phase === "applying") {
      const pending = review.rows.find(
        (row) =>
          row.kind === "account" &&
          row.selected &&
          row.available &&
          review.includeAccounts &&
          !review.accountReceipts[row.shared!]?.committed,
      );
      if (pending) {
        await this.applyAccount(review, pending);
        return;
      }
      if (review.includeSettings && !review.settingsReceipt) {
        const changes: ApplyRequest["changes"] = {};
        for (const row of review.rows)
          if (row.kind === "setting" && row.selected && row.available)
            changes[row.key as BrowserSettingKey] = row.reset
              ? defaultPortable(row.key as BrowserSettingKey)
              : row.value;
        const request: ApplyRequest = review.settingsRequest ?? {
          id: review.id,
          baseline: review.baseline,
          changes,
        };
        if (!review.settingsRequest) {
          review.settingsRequest = request;
          await this.save();
        }
        const receipt = this.device.apply(request);
        if (receipt.id !== review.id)
          throw new Error(
            "The saved preference receipt belongs to another review. Reopen this profile.",
          );
        review.settingsReceipt = receipt;
        review.applied += receipt.applied.length;
        review.kept += receipt.kept.length;
        await this.save();
        this.device.acknowledge(receipt.id);
        this.changed();
        return;
      }
      review.phase = "complete";
      await this.save();
      this.changed();
    }
  }
  /// Reserve the local identity durably, commit the account in the mail
  /// store, then record the receipt; a lost reply reuses the same identity.
  private async applyAccount(review: EnrollmentReview, row: EnrollmentRow) {
    const shared = row.shared!;
    const reserved = review.accountReceipts[shared];
    const local = reserved?.local ?? row.local?.id ?? crypto.randomUUID();
    if (!reserved) {
      review.accountReceipts[shared] = { local, committed: false };
      await this.save();
    }
    const nameRow = review.rows.find(
      (r) =>
        r.kind === "name" && r.shared === shared && r.selected && r.available,
    );
    const existing = this.device.accounts().find((a) => a.id === local);
    await this.device.importAccount(
      localAccount(local, row.account!, nameRow?.name ?? null, existing),
    );
    await this.store.commit([
      {
        store: "mappings",
        key: local,
        value: {
          local,
          shared,
          source: "enrollment",
          // Imported credentials never exist; a kept local account keeps its own.
          reconnect: !row.local,
        } satisfies AccountMapping,
      },
    ]);
    review.accountReceipts[shared] = { local, committed: true };
    review.applied++;
    await this.save();
    this.changed();
  }
  private async buildRows(journal: ProfileJournal): Promise<EnrollmentRow[]> {
    const rows: EnrollmentRow[] = [];
    const mappings = new Map<string, AccountMapping>();
    for (const row of await this.store.range<AccountMapping>(
      "mappings",
      "",
      "￿",
      Number.MAX_SAFE_INTEGER,
    ))
      mappings.set(row.value.shared, row.value);
    const accounts = this.device.accounts();
    let after: string | null = null;
    for (;;) {
      const fields: Field[] = await journal.execute(
        { kind: "fields", after },
        "fields",
      );
      for (const field of fields) {
        const position = rows.length;
        if (field.conflict) {
          rows.push({
            position,
            target: field.target,
            kind: field.target.startsWith("setting:") ? "setting" : "account",
            available: false,
            reason: "Concurrent versions need review on the publishing device.",
            selected: false,
            key: field.target.startsWith("setting:")
              ? (field.target.slice("setting:".length) as SettingKey)
              : undefined,
          });
          continue;
        }
        const versions = await journal.execute(
          { kind: "versions", target: field.target, after: null },
          "versions",
        );
        if (versions.length !== 1) continue;
        const change: Change = await journal.execute(
          {
            kind: "value",
            target: field.target,
            operation: versions[0].operation,
          },
          "value",
        );
        if (change.kind === "account_connection") {
          const mapping = mappings.get(change.account.id);
          const local = mapping
            ? accounts.find((a) => a.id === mapping.local)
            : undefined;
          const extensions = Object.keys(change.account).filter(
            (k) =>
              !(k in localAccount("x", change.account, null)) && k !== "id",
          );
          const changed = local
            ? !sameConnection(local, change.account)
            : false;
          rows.push({
            position,
            target: field.target,
            kind: "account",
            shared: change.account.id,
            account: change.account,
            local,
            available: !extensions.length,
            reason: extensions.length
              ? "This connection uses settings this browser cannot preserve. Update Shep first."
              : changed
                ? "The shared connection differs from the account on this browser; applying adds a separate account."
                : local
                  ? "Keeps this browser's cached mail and credentials."
                  : "Added without a password; reconnect to activate it.",
            selected: !extensions.length && !changed,
          });
          if (changed) {
            // A changed connection never reuses the local identity or credentials.
            rows[rows.length - 1].local = undefined;
          }
        } else if (change.kind === "account_name") {
          rows.push({
            position,
            target: field.target,
            kind: "name",
            shared: change.id,
            name: change.name,
            available: true,
            selected: true,
          });
        } else if (
          change.kind === "setting" ||
          change.kind === "setting_removed"
        ) {
          const supported = (BROWSER_SETTINGS as readonly string[]).includes(
            change.key,
          );
          rows.push({
            position,
            target: field.target,
            kind: "setting",
            key: change.key,
            value: change.kind === "setting" ? change.value : undefined,
            reset: change.kind === "setting_removed",
            available: supported,
            reason: supported ? undefined : "Not used by the browser client.",
            selected: supported,
          });
        }
      }
      if (fields.length < 50) break;
      after = fields[fields.length - 1].target;
    }
    // Name rows without an available account row cannot apply.
    for (const row of rows)
      if (row.kind === "name") {
        const owner = rows.find(
          (r) => r.kind === "account" && r.shared === row.shared,
        );
        if (!owner || !owner.available) {
          row.available = false;
          row.selected = false;
          row.reason = "Its account definition is unavailable.";
        }
      }
    return rows;
  }
  dispose() {
    this.invalidate();
  }
}

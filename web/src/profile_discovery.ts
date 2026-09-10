// Durable browser profile discovery, mirroring the native catalog
// (docs/agents/PROFILE_DISCOVERY.md): capture a change token, list and stage
// pages, verify every file against its exact media, import originals into a
// per-profile observation journal, then replay changes. Saved progress
// survives reloads; a failed or incomplete scan is never an empty profile.
import type { ProfileGoogleApi } from "./profile_google";
import {
  ProfileJournal,
  validateRecord,
  type HistoryPort,
} from "./profile_history";
import {
  fileName,
  parseChangesPage,
  parseListPage,
  parseMedia,
  parseStartPageToken,
  parseDriveFile,
  sha256Hex,
  verifyFileMetadata,
  verifyMedia,
  type DriveFile,
} from "./profile_drive";
import type {
  CatalogFile,
  ProfileStore,
  ProfileSummary,
  ScanState,
} from "./profile_store";
import {
  bindingKey,
  type Binding,
  type Operation,
  type StoredRecord,
} from "./profile_types";

export interface DiscoveryScope {
  namespace: string;
  principal: string;
}
const idle = (): ScanState => ({
  revision: 0,
  phase: "idle",
  failed: false,
  changeToken: null,
  pageToken: null,
  visited: [],
  listed: [],
  pending: [],
  pendingIndex: 0,
  error: null,
  completedRevision: null,
  scanned: 0,
});
export class ProfileDiscovery extends EventTarget {
  state: ScanState = idle();
  busy = false;
  paused = false;
  page: ProfileSummary[] = [];
  private pageAfter: string | null = null;
  private generation = 0;
  private journals = new Map<string, ProfileJournal>();
  private namespaceDigest: Promise<string>;
  constructor(
    readonly store: ProfileStore,
    private api: ProfileGoogleApi,
    private port: HistoryPort,
    readonly scope: DiscoveryScope,
  ) {
    super();
    this.namespaceDigest = sha256Hex(scope.namespace);
  }
  get complete() {
    return (
      this.state.phase === "complete" &&
      this.state.completedRevision === this.state.revision
    );
  }
  get error() {
    return this.state.error;
  }
  private changed() {
    this.dispatchEvent(new Event("change"));
  }
  /// Restore saved progress and the first summary page.
  async load() {
    this.state = (await this.store.get<ScanState>("scan", "state")) ?? idle();
    await this.firstPage();
  }
  /// Fence every in-flight step after a Google change; saved progress stays.
  invalidate() {
    this.generation++;
    this.busy = false;
    for (const journal of this.journals.values()) void journal.close();
    this.journals.clear();
    this.changed();
  }
  async firstPage() {
    this.pageAfter = null;
    this.page = await this.store.profilesPage(null);
    this.changed();
  }
  async nextPage() {
    const last = this.page.at(-1);
    if (!last) return;
    this.pageAfter = `${last.profile} ${last.generation}`;
    this.page = await this.store.profilesPage(this.pageAfter);
    this.changed();
  }
  /// Full refresh: a new revision rejects old data and errors.
  find() {
    return this.start({
      ...idle(),
      revision: this.state.revision + 1,
      phase: "listing",
    });
  }
  /// Continue from the saved step after a failure or pause; the working
  /// phase and every verified receipt are retained.
  retry() {
    if (this.state.phase === "idle") return this.find();
    return this.start({ ...this.state, failed: false, error: null });
  }
  pause() {
    this.paused = true;
    this.changed();
  }
  async resume() {
    this.paused = false;
    if (!this.busy && !this.complete) await this.retry();
    else this.changed();
  }
  private async start(state: ScanState) {
    if (this.busy) return;
    this.busy = true;
    this.paused = false;
    const generation = this.generation;
    this.state = state;
    try {
      await this.save();
      this.changed();
      while (
        this.generation === generation &&
        !this.paused &&
        this.state.phase !== "complete" &&
        !this.state.failed
      ) {
        await this.step(generation);
      }
    } catch (error) {
      if (this.generation !== generation) return;
      this.state = {
        ...this.state,
        failed: true,
        error:
          error instanceof Error
            ? error.message
            : "Discovery failed. Retry the same step.",
      };
      try {
        await this.save();
      } catch {
        // The saved error is best effort; the in-memory state still shows it.
      }
    } finally {
      if (this.generation === generation) {
        this.busy = false;
        this.changed();
      }
    }
  }
  private save() {
    return this.store.commit([
      { store: "scan", key: "state", value: this.state },
    ]);
  }
  private async step(generation: number) {
    const s = this.state;
    switch (s.phase) {
      case "idle":
        this.state = { ...s, phase: "listing" };
        return;
      case "listing": {
        if (s.changeToken === null) {
          const token = parseStartPageToken(
            await this.api.drive({ op: "start_page_token" }),
          );
          this.fence(generation, s.revision);
          this.state = { ...s, changeToken: token };
          await this.save();
          return;
        }
        const page = parseListPage(
          await this.api.drive(
            s.pageToken
              ? { op: "list", page_token: s.pageToken }
              : { op: "list" },
          ),
        );
        this.fence(generation, s.revision);
        const listed = [...s.listed];
        const pending = [...s.pending];
        for (const file of page.files) {
          if (listed.includes(file.id))
            throw new Error(
              "Drive listed the same profile file twice. Retry discovery.",
            );
          listed.push(file.id);
          pending.push(file.id);
        }
        const visited = [...s.visited, s.pageToken ?? ""];
        if (page.nextPageToken !== null) {
          if (visited.includes(page.nextPageToken) || visited.length > 10_000)
            throw new Error(
              "Drive returned a repeating listing. Retry discovery later.",
            );
          this.state = {
            ...s,
            listed,
            pending,
            visited,
            pageToken: page.nextPageToken,
          };
          await this.save();
          return;
        }
        // Known files must still be present after a full listing.
        const known = await this.store.range<CatalogFile>(
          "catalog",
          "",
          "￿",
          Number.MAX_SAFE_INTEGER,
        );
        const missing = known.filter((row) => !listed.includes(row.key));
        if (missing.length)
          throw new Error(
            `${missing.length} previously verified profile ${missing.length === 1 ? "file is" : "files are"} no longer listed. Review the Google account before trusting this catalog.`,
          );
        this.state = {
          ...s,
          listed,
          pending,
          visited,
          pageToken: null,
          phase: "verifying",
        };
        await this.save();
        return;
      }
      case "verifying": {
        if (s.pendingIndex >= s.pending.length) {
          this.state = { ...s, phase: "replaying" };
          await this.save();
          return;
        }
        const id = s.pending[s.pendingIndex];
        await this.verify(generation, id);
        this.fence(generation, s.revision);
        this.state = {
          ...this.state,
          pendingIndex: s.pendingIndex + 1,
          scanned: s.scanned + 1,
        };
        await this.save();
        return;
      }
      case "replaying": {
        const page = parseChangesPage(
          await this.api.drive({ op: "changes", page_token: s.changeToken! }),
        );
        this.fence(generation, s.revision);
        const pending = [...s.pending];
        const listed = [...s.listed];
        for (const change of page.changes) {
          const known = await this.store.get<CatalogFile>(
            "catalog",
            change.fileId,
          );
          if (change.removed) {
            if (known)
              throw new Error(
                "A verified profile file was removed from Google Drive. Review the account before trusting this catalog.",
              );
            continue;
          }
          if (!change.file) continue;
          if (!listed.includes(change.fileId)) {
            listed.push(change.fileId);
            pending.push(change.fileId);
          } else if (!known && !pending.includes(change.fileId))
            pending.push(change.fileId);
        }
        if (pending.length > s.pendingIndex) {
          this.state = {
            ...s,
            pending,
            listed,
            phase: "verifying",
            changeToken: page.nextPageToken ?? s.changeToken,
          };
          await this.save();
          return;
        }
        if (page.nextPageToken) {
          if (page.nextPageToken === s.changeToken)
            throw new Error("Drive repeated a change token. Retry discovery.");
          this.state = {
            ...s,
            pending,
            listed,
            changeToken: page.nextPageToken,
          };
          await this.save();
          return;
        }
        this.state = {
          ...s,
          pending,
          listed,
          changeToken: page.newStartPageToken,
          phase: "complete",
          completedRevision: s.revision,
          error: null,
        };
        await this.save();
        await this.firstPage();
        return;
      }
    }
  }
  private fence(generation: number, revision: number) {
    if (this.generation !== generation || this.state.revision !== revision)
      throw new Error("Discovery was restarted; this result was discarded.");
  }
  /// Verify metadata, exact media and decoded identity, then import the
  /// original into the profile's observation journal and save the receipt.
  private async verify(generation: number, id: string) {
    const known = await this.store.get<CatalogFile>("catalog", id);
    const metadata = await this.api.drive({ op: "metadata", file_id: id });
    this.fence(generation, this.state.revision);
    const file = parseDriveFile(metadata);
    if (!file)
      throw new Error(
        known
          ? "A verified profile file lost its profile marker. Review the Google account before trusting this catalog."
          : "A listed profile file is no longer readable. Retry discovery.",
      );
    const identity = verifyFileMetadata(file, await this.namespaceDigest);
    if (
      known &&
      (known.operation !== identity.operation ||
        known.sha256 !== identity.sha256)
    )
      throw new Error(
        "A verified profile file changed identity. Review the Google account before trusting this catalog.",
      );
    const media = parseMedia(
      await this.api.drive({ op: "media", file_id: id }),
    );
    this.fence(generation, this.state.revision);
    const normalised = await validateRecord(this.port, media);
    const decoded = JSON.parse(normalised) as Operation;
    if (decoded.namespace !== this.scope.namespace)
      throw new Error(
        "This profile belongs to a different Shep application namespace.",
      );
    await verifyMedia(file, identity, media, decoded);
    const binding: Binding = {
      namespace: this.scope.namespace,
      principal: this.scope.principal,
      profile: identity.profile,
      generation: identity.generation,
    };
    const journal = await this.journal(binding);
    // Catalog identity first, then the immutable import, then the summary.
    await this.store.commit([
      {
        store: "catalog",
        key: id,
        value: {
          id,
          name: fileName(identity.operation),
          operation: identity.operation,
          profile: identity.profile,
          generation: identity.generation,
          sha256: identity.sha256,
          size: file.size,
          origin: known?.origin ?? "discovered",
        } satisfies CatalogFile,
      },
    ]);
    await this.importRecord(journal, media);
    await this.summarise(journal);
  }
  async journal(binding: Binding): Promise<ProfileJournal> {
    const key = `observe:${bindingKey(binding)}`;
    const existing = this.journals.get(key);
    if (existing) return existing;
    const device = await this.store.deviceFor(key);
    const journal = await ProfileJournal.open(
      this.port,
      key,
      binding,
      device,
      await this.store.records(key),
    );
    this.journals.set(key, journal);
    return journal;
  }
  /// Import one original and drain bounded batches until nothing is ready.
  async importRecord(journal: ProfileJournal, record: string) {
    const operation = (JSON.parse(record) as Operation).operation;
    let state = await journal.execute({ kind: "import", record }, "state");
    const stored: StoredRecord = await journal.record(operation);
    await this.store.saveRecord(journal.key, stored);
    while (state.ready > 0)
      state = await journal.execute({ kind: "drain" }, "state");
    return state;
  }
  async summarise(journal: ProfileJournal): Promise<ProfileSummary> {
    const overview = await journal.overview();
    const files = (
      await this.store.range<CatalogFile>(
        "catalog",
        "",
        "￿",
        Number.MAX_SAFE_INTEGER,
      )
    ).filter(
      (row) =>
        row.value.profile === journal.binding.profile &&
        row.value.generation === journal.binding.generation,
    ).length;
    const summary: ProfileSummary = {
      profile: journal.binding.profile,
      generation: journal.binding.generation,
      name: overview.name,
      nameConflict: overview.name_conflict,
      accounts: overview.accounts,
      settings: overview.settings,
      initialized: overview.state.initialized,
      waiting: overview.state.waiting,
      ready: overview.state.ready,
      conflicts: overview.state.conflicts,
      removed: overview.state.removed,
      files,
      revision: overview.state.revision,
    };
    await this.store.commit([
      {
        store: "profiles",
        key: `${summary.profile} ${summary.generation}`,
        value: summary,
      },
    ]);
    return summary;
  }
  /// Files verified for one profile, for enrollment source checks.
  async filesFor(profile: string, generation: string): Promise<DriveFile[]> {
    return (
      await this.store.range<CatalogFile>(
        "catalog",
        "",
        "￿",
        Number.MAX_SAFE_INTEGER,
      )
    )
      .filter(
        (row) =>
          row.value.profile === profile && row.value.generation === generation,
      )
      .map((row) => ({
        id: row.value.id,
        name: row.value.name,
        mimeType: "application/json",
        size: row.value.size,
        trashed: false,
        ownedByMe: true,
        appProperties: {},
      }));
  }
  dispose() {
    this.invalidate();
  }
}

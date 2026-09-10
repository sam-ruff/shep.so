import type { Database } from "@sqlite.org/sqlite-wasm";
import { BulkJournal, type BulkItem, type BulkJob } from "./bulk_journal";
import { read, walk } from "./mailbox_cache";
import type { CacheChange, CacheState, CacheMail } from "./cache_changes";
import type { MailIntent } from "./mail_intents";
import type { MailAlias } from "./sent_cache";

const schema = `
CREATE TABLE IF NOT EXISTS bulk_index_state(singleton INTEGER PRIMARY KEY,epoch TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS bulk_index_jobs(id TEXT PRIMARY KEY,revision INTEGER NOT NULL,data TEXT NOT NULL,seen INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS bulk_index_items(job TEXT NOT NULL,position INTEGER NOT NULL,id TEXT NOT NULL,data TEXT NOT NULL,PRIMARY KEY(job,position));
CREATE INDEX IF NOT EXISTS bulk_index_identity ON bulk_index_items(id);
CREATE TABLE IF NOT EXISTS bulk_source_state(singleton INTEGER PRIMARY KEY,epoch TEXT NOT NULL,revision INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS bulk_source_intents(id TEXT PRIMARY KEY,data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS bulk_source_aliases(alias TEXT PRIMARY KEY,target TEXT NOT NULL,lineage TEXT,target_lineage TEXT);
CREATE TABLE IF NOT EXISTS bulk_source_lineages(id TEXT PRIMARY KEY,lineage TEXT,cached_folder TEXT,cached_unread INTEGER,cached_starred INTEGER);
CREATE TEMP TABLE IF NOT EXISTS bulk_source_accounts(id TEXT PRIMARY KEY);
CREATE TEMP TABLE IF NOT EXISTS bulk_current(id TEXT PRIMARY KEY,folder TEXT,unread INTEGER,starred INTEGER);
`;

/** Both mailbox queries and captured selections use these same full-group
 * effects. Only changed durable metadata crosses into worker-owned SQLite. */
export class BulkProjection {
  constructor(
    private sql: Database,
    private user: string,
  ) {
    sql.exec(schema);
    if (
      !sql.selectValue(
        "SELECT 1 FROM pragma_table_info('bulk_source_aliases') WHERE name='lineage'",
      )
    ) {
      sql.exec(
        "ALTER TABLE bulk_source_aliases ADD COLUMN lineage TEXT; ALTER TABLE bulk_source_aliases ADD COLUMN target_lineage TEXT; DELETE FROM bulk_source_state;",
      );
    }
    if (
      !sql.selectValue(
        "SELECT 1 FROM pragma_table_info('bulk_source_lineages') WHERE name='cached_folder'",
      )
    ) {
      sql.exec(
        "ALTER TABLE bulk_source_lineages ADD COLUMN cached_folder TEXT; ALTER TABLE bulk_source_lineages ADD COLUMN cached_unread INTEGER; ALTER TABLE bulk_source_lineages ADD COLUMN cached_starred INTEGER; DELETE FROM bulk_source_state;",
      );
    }
  }
  private exec(sql: string, bind: (string | number | null)[] = []) {
    this.sql.exec({ sql, bind: bind.length ? bind : undefined });
  }
  async journal() {
    return BulkJournal.inspect(this.user, (journal) =>
      journal.projection({
        begin: (_revision, epoch) => {
          if (
            this.sql.selectValue(
              "SELECT epoch FROM bulk_index_state WHERE singleton=1",
            ) !== epoch
          ) {
            this.exec(
              "DELETE FROM bulk_index_jobs; DELETE FROM bulk_index_items;",
            );
            this.exec(
              "INSERT INTO bulk_index_state VALUES(1,?) ON CONFLICT(singleton) DO UPDATE SET epoch=excluded.epoch",
              [epoch],
            );
          }
          this.exec("UPDATE bulk_index_jobs SET seen=0");
        },
        job: (job: BulkJob) => {
          const previous = this.sql.selectValue(
            "SELECT revision FROM bulk_index_jobs WHERE id=?",
            [job.id],
          );
          // Staging and retiring rows are invisible; the complete review is
          // copied once it is ready.
          const stage = ["preparing", "interrupted", "cancelled"].includes(
            job.state,
          );
          this.exec(
            "INSERT INTO bulk_index_jobs VALUES(?,?,?,1) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,data=excluded.data,seen=1",
            [job.id, stage ? -1 : job.revision, JSON.stringify(job)],
          );
          return stage
            ? job.revision
            : typeof previous === "number"
              ? previous
              : -1;
        },
        item: (item: BulkItem) =>
          this.exec(
            "INSERT INTO bulk_index_items VALUES(?,?,?,?) ON CONFLICT(job,position) DO UPDATE SET id=excluded.id,data=excluded.data",
            [item.job, item.position, item.id, JSON.stringify(item)],
          ),
        end: () => {
          this.exec("DELETE FROM bulk_index_jobs WHERE seen=0");
          this.exec(
            "DELETE FROM bulk_index_items WHERE job NOT IN(SELECT id FROM bulk_index_jobs)",
          );
        },
      }),
    );
  }
  /** Only derived SQLite is changed, inside the caller's query transaction.
   * A preview never reserves intent, changes the journal, or issues provider work. */
  previewUndo(id: string) {
    const data = this.sql.selectValue(
      "SELECT data FROM bulk_index_jobs WHERE id=?",
      [id],
    );
    if (typeof data !== "string")
      throw Error("This group is no longer available. Refresh History.");
    const job = JSON.parse(data) as BulkJob;
    if (!["review", "ready"].includes(job.state))
      throw Error(
        "This group review is incomplete. Select its messages again.",
      );
    if (!job.undo && job.state === "ready") {
      job.undo = true;
      job.undoIntent = Number.MAX_SAFE_INTEGER;
      this.exec("UPDATE bulk_index_jobs SET data=? WHERE id=?", [
        JSON.stringify(job),
        id,
      ]);
    }
    this.materialize();
    let restored = false;
    return {
      id,
      revision: job.revision,
      committed: (JSON.parse(data) as BulkJob).undo,
      restore: () => {
        if (restored) return;
        restored = true;
        this.exec("UPDATE bulk_index_jobs SET data=? WHERE id=?", [data, id]);
        this.materialize();
      },
    };
  }
  materialize() {
    this.exec("DELETE FROM bulk_current");
    this.exec(
      `INSERT INTO bulk_current WITH ${bulkEffects} SELECT id,CASE WHEN lower(folder)='inbox' THEN 'INBOX' ELSE folder END,unread,starred FROM bulk_effect`,
    );
  }
  fields(id: string) {
    const row = this.sql.selectObject(
      "SELECT folder,unread,starred FROM bulk_current WHERE id=?",
      [id],
    );
    return row
      ? {
          ...(row.folder !== null ? { folder: row.folder as string } : {}),
          ...(row.unread !== null ? { unread: !!row.unread } : {}),
          ...(row.starred !== null ? { starred: !!row.starred } : {}),
        }
      : {};
  }
  private async intent(tx: IDBTransaction, id: string) {
    const value = await read<MailIntent | undefined>(
      tx.objectStore("mailIntents").get(id),
    );
    if (value)
      this.exec(
        "INSERT INTO bulk_source_intents VALUES(?,?) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        [id, JSON.stringify(value)],
      );
    else this.exec("DELETE FROM bulk_source_intents WHERE id=?", [id]);
  }
  private async lineage(tx: IDBTransaction, id: string) {
    const value = await read<CacheMail | undefined>(
      tx.objectStore("mailMetadata").get(id),
    );
    if (value)
      this.exec(
        "INSERT INTO bulk_source_lineages VALUES(?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET lineage=excluded.lineage,cached_folder=excluded.cached_folder,cached_unread=excluded.cached_unread,cached_starred=excluded.cached_starred",
        [
          id,
          value.lineage ?? null,
          value.core.folder,
          +value.core.unread,
          +value.core.starred,
        ],
      );
    else this.exec("DELETE FROM bulk_source_lineages WHERE id=?", [id]);
  }
  async source(
    tx: IDBTransaction,
    state: CacheState & { epoch: string },
    accounts: { id: string }[],
  ) {
    const previous = this.sql.selectObject(
      "SELECT epoch,revision FROM bulk_source_state WHERE singleton=1",
    ) as { epoch: string; revision: number } | undefined;
    let incremental =
      previous?.epoch === state.epoch && previous.revision === state.revision;
    if (
      previous?.epoch === state.epoch &&
      previous.revision >= state.floor &&
      previous.revision < state.revision &&
      state.revision - previous.revision <= 1024
    ) {
      const changes = await read<CacheChange[]>(
        tx
          .objectStore("mailChanges")
          .getAll(
            IDBKeyRange.bound(previous.revision, state.revision, true),
            1024,
          ),
      );
      incremental =
        changes.length === state.revision - previous.revision &&
        changes.every(
          (c, i) =>
            c.revision === previous.revision + i + 1 &&
            !c.reset &&
            typeof c.id === "string",
        );
      if (incremental)
        for (const id of new Set(changes.map((c) => c.id!))) {
          await this.intent(tx, id);
          await this.lineage(tx, id);
          const alias = await read<MailAlias | undefined>(
            tx.objectStore("mailAliases").get(id),
          );
          if (alias) {
            this.exec(
              "INSERT INTO bulk_source_aliases VALUES(?,?,?,?) ON CONFLICT(alias) DO UPDATE SET target=excluded.target,lineage=excluded.lineage,target_lineage=excluded.target_lineage",
              [
                id,
                alias.target,
                alias.lineage ?? null,
                alias.targetLineage ?? null,
              ],
            );
            await this.intent(tx, alias.target);
            await this.lineage(tx, alias.target);
          } else
            this.exec("DELETE FROM bulk_source_aliases WHERE alias=?", [id]);
        }
    }
    if (!incremental) {
      this.exec(
        "DELETE FROM bulk_source_intents; DELETE FROM bulk_source_aliases; DELETE FROM bulk_source_lineages;",
      );
      const intent = this.sql.prepare(
          "INSERT INTO bulk_source_intents VALUES(?,?)",
        ),
        alias = this.sql.prepare(
          "INSERT INTO bulk_source_aliases VALUES(?,?,?,?)",
        ),
        lineage = this.sql.prepare(
          "INSERT INTO bulk_source_lineages VALUES(?,?,?,?,?)",
        );
      try {
        await walk(tx.objectStore("mailIntents").openCursor(), (row) => {
          intent
            .bind([String(row.primaryKey), JSON.stringify(row.value)])
            .stepReset();
        });
        await walk(tx.objectStore("mailAliases").openCursor(), (row) => {
          const value = row.value as MailAlias;
          alias
            .bind([
              String(row.primaryKey),
              value.target,
              value.lineage ?? null,
              value.targetLineage ?? null,
            ])
            .stepReset();
        });
        await walk(tx.objectStore("mailMetadata").openCursor(), (row) => {
          const value = row.value as CacheMail;
          lineage
            .bind([
              String(row.primaryKey),
              value.lineage ?? null,
              value.core.folder,
              +value.core.unread,
              +value.core.starred,
            ])
            .stepReset();
        });
      } finally {
        lineage.finalize();
        intent.finalize();
        alias.finalize();
      }
    }
    this.exec("DELETE FROM bulk_source_accounts");
    // Account removal already participates in this readonly mail snapshot.
    // Do not add removedAccounts: draft saves check its fence in a readwrite
    // transaction and must remain independent of a long mail capture/rebuild.
    for (const account of accounts)
      this.exec("INSERT INTO bulk_source_accounts VALUES(?)", [account.id]);
    this.exec(
      "INSERT INTO bulk_source_state VALUES(1,?,?) ON CONFLICT(singleton) DO UPDATE SET epoch=excluded.epoch,revision=excluded.revision",
      [state.epoch, state.revision],
    );
  }
}

/** Receipt/cache races are value projections, never physical UID changes.
 * Applied cache revisions prevent a completed job from overriding later sync;
 * ownership revisions preserve newer choices even when their values match. */
export const bulkEffects = `
bulk_candidates AS (
 SELECT COALESCE(a.target,i.id) id,i.data item,j.data job,s.data intent,l.cached_folder,l.cached_unread,l.cached_starred,
 json_extract(j.data,'$.forwardIntent') forward_revision,
 json_extract(j.data,'$.undoIntent') undo_revision,
 json_extract(j.data,'$.undo') undo
 FROM bulk_index_items i JOIN bulk_index_jobs j ON j.id=i.job
 LEFT JOIN bulk_source_aliases a ON a.alias=i.id
 LEFT JOIN bulk_source_intents s ON s.id=COALESCE(a.target,i.id)
 LEFT JOIN bulk_source_lineages l ON l.id=COALESCE(a.target,i.id)
 WHERE json_extract(j.data,'$.state')='ready'
 AND (json_extract(i.data,'$.original.lineage') IS NULL OR l.lineage=json_extract(i.data,'$.original.lineage') OR (a.lineage=json_extract(i.data,'$.original.lineage') AND a.target_lineage=l.lineage))
 AND json_extract(j.data,'$.cacheEpoch')=(SELECT epoch FROM bulk_source_state WHERE singleton=1)
 AND NOT EXISTS(SELECT 1 FROM json_each(json_extract(i.data,'$.owners')) o WHERE NOT EXISTS(SELECT 1 FROM bulk_source_accounts a WHERE a.id=o.value))
),
bulk_fields AS (
 SELECT c.id,item,job,intent,forward_revision,undo_revision,undo,f.value field,
 CASE WHEN undo THEN COALESCE(json_extract(item,'$.receipt.before.'||f.value), CASE WHEN json_extract(item,'$.status')='running' THEN CASE f.value WHEN 'folder' THEN cached_folder WHEN 'unread' THEN cached_unread WHEN 'starred' THEN cached_starred END ELSE json_extract(item,'$.original.'||f.value) END)
 ELSE json_extract(job,'$.action.'||f.value) END value,
 CASE WHEN undo THEN undo_revision ELSE forward_revision END revision,
 json_extract(intent,'$.fields.'||f.value||'.revision') owner_revision,
 json_extract(intent,'$.fields.'||f.value||'.origin') owner_origin,
 json_extract(intent,'$.fields.'||f.value||'.status') owner_status,
 COALESCE(json_extract(intent,'$.applied.'||f.value),0) applied_revision
 FROM bulk_candidates c CROSS JOIN json_each('["folder","unread","starred"]') f
 WHERE (json_extract(item,'$.intent') IS NULL AND json_extract(job,'$.action.'||f.value) IS NOT NULL)
 OR json_extract(item,'$.intent.fields.'||f.value) IS NOT NULL
),
bulk_ranked AS (
 SELECT id,field,value,revision,ROW_NUMBER() OVER(PARTITION BY id,field ORDER BY revision DESC,json_extract(job,'$.created') DESC,json_extract(job,'$.id') DESC) rank
 FROM bulk_fields
 WHERE value IS NOT NULL AND revision>0 AND applied_revision<revision AND (
 (NOT undo AND COALESCE(owner_revision,0)<=forward_revision
  AND json_extract(item,'$.status') IN('pending','running','done')
  AND NOT(COALESCE(owner_revision=forward_revision AND owner_status='failed' AND json_extract(item,'$.status')='running',0)))
 OR (undo AND (owner_revision=forward_revision OR (owner_revision=undo_revision AND owner_origin=forward_revision))
  AND json_extract(item,'$.status') IN('running','done','undo_running','restored'))
)),
bulk_effect AS (
 SELECT id,MAX(CASE WHEN field='folder' THEN value END) folder,
 MAX(CASE WHEN field='unread' THEN value END) unread,MAX(CASE WHEN field='starred' THEN value END) starred
 FROM bulk_ranked WHERE rank=1 GROUP BY id
)`;

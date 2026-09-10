import { BulkProjection } from "./bulk_projection";
import sqliteInit, {
  type Database,
  type Sqlite3Static,
  type SAHPoolUtil,
  type PreparedStatement,
} from "@sqlite.org/sqlite-wasm";
import sqliteWasm from "@sqlite.org/sqlite-wasm/sqlite3.wasm?url";
import { openMailDatabase } from "./storage";
import { read, walk, snapshot, sourceState, display } from "./mailbox_cache";
import { inboxFolder, senderName } from "./mail_query";
import { roleFolders, type MailAlias, type MailRoles } from "./sent_cache";
import type { Account, CoreMail, RecordMail } from "./provider";
import type { CacheState, CacheChange } from "./cache_changes";
import type { Mail } from "./model";
import {
  checkedQuery,
  type MailboxQuery,
  type MailboxPage,
} from "./mailbox_types";

type Bind = (string | number | null)[];
const sourceStores = [
  "accounts",
  "mail",
  "mailAliases",
  "mailRoles",
  "cacheState",
  "mailChanges",
  "mailIntents",
  "mailMetadata",
];
const triggers = `CREATE TRIGGER IF NOT EXISTS messages_insert AFTER INSERT ON messages BEGIN
  INSERT INTO terms(rowid,search) VALUES(new.rowid,new.search);
END;
CREATE TRIGGER IF NOT EXISTS messages_delete AFTER DELETE ON messages BEGIN
  INSERT INTO terms(terms,rowid,search) VALUES('delete',old.rowid,old.search);
END;
CREATE TRIGGER IF NOT EXISTS messages_update AFTER UPDATE OF search ON messages WHEN old.search IS NOT new.search BEGIN
  INSERT INTO terms(terms,rowid,search) VALUES('delete',old.rowid,old.search);
  INSERT INTO terms(rowid,search) VALUES(new.rowid,new.search);
END;
`;
const insertMail = `INSERT INTO messages(id,account,folder,unread,starred,timestamp,core,search) VALUES(?,?,?,?,?,?,?,?)
 ON CONFLICT(id) DO UPDATE SET account=excluded.account,folder=excluded.folder,unread=excluded.unread,starred=excluded.starred,timestamp=excluded.timestamp,core=excluded.core,search=excluded.search`;
const schema = `
PRAGMA journal_mode=DELETE;
PRAGMA synchronous=FULL;
PRAGMA cache_size=-8192;
PRAGMA temp_store=FILE;
CREATE TABLE IF NOT EXISTS checkpoint(singleton INTEGER PRIMARY KEY CHECK(singleton=1), epoch TEXT NOT NULL, revision INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS messages(rowid INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE, account TEXT NOT NULL, folder TEXT NOT NULL, unread INTEGER NOT NULL, starred INTEGER NOT NULL, timestamp REAL NOT NULL, core TEXT NOT NULL, search TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS newest ON messages(folder,timestamp DESC,id);
CREATE INDEX IF NOT EXISTS oldest ON messages(folder,timestamp,id);
CREATE INDEX IF NOT EXISTS account_folder ON messages(account,folder);
CREATE TABLE IF NOT EXISTS aliases(alias TEXT PRIMARY KEY,target TEXT NOT NULL);
CREATE VIRTUAL TABLE IF NOT EXISTS terms USING fts5(search, content='messages', content_rowid='rowid', tokenize='trigram case_sensitive 1');
CREATE TEMP TABLE accounts(id TEXT PRIMARY KEY,email TEXT NOT NULL);
CREATE TEMP TABLE sent(account TEXT NOT NULL,folder TEXT NOT NULL,PRIMARY KEY(account,folder));
CREATE TEMP TABLE pending(id TEXT PRIMARY KEY,folder TEXT,unread INTEGER,starred INTEGER);`;

/** Derived, per-profile search storage. IndexedDB remains authoritative. Every
 * request owns a Web Lock from VFS installation/acquisition through DB close
 * and pause, so idle tabs retain no exclusive OPFS access handles. */
export class MailboxStore {
  private pool?: SAHPoolUtil;
  private sql?: Database;
  private closed = false;
  private accountSet = new Set<string>();
  private insert?: PreparedStatement;
  private constructor(
    private cache: IDBDatabase,
    private sqlite: Sqlite3Static,
    private user: string,
  ) {}
  static async open(user: string) {
    const cache = await openMailDatabase(user);
    try {
      const initialize = sqliteInit as unknown as (options: {
        locateFile: (name: string) => string;
      }) => Promise<Sqlite3Static>;
      return new MailboxStore(
        cache,
        await initialize({ locateFile: () => sqliteWasm }),
        user,
      );
    } catch (error) {
      cache.close();
      throw error;
    }
  }
  close() {
    this.closed = true;
    this.cache.close();
  }
  private exec(sql: string, bind: Bind = []) {
    this.sql!.exec({ sql, bind: bind.length ? bind : undefined });
  }
  private value(sql: string, bind: Bind = []) {
    return Number(
      this.sql!.selectValue(sql, bind.length ? bind : undefined) ?? 0,
    );
  }
  private canonical(id: string): string {
    return (
      (this.sql!.selectValue("SELECT target FROM aliases WHERE alias=?", [
        id,
      ]) as string | undefined) ?? id
    );
  }
  private put(id: string, mail: RecordMail | undefined) {
    if (!mail || mail.moved || !this.accountSet.has(mail.core.account_id)) {
      this.exec("DELETE FROM messages WHERE id=?", [id]);
      return;
    }
    const m = mail.core;
    if (!Number.isFinite(m.timestamp) || typeof mail.text !== "string")
      throw Error(
        "A cached message could not be indexed. Refresh its account and retry.",
      );
    this.insert!.bind([
      id,
      m.account_id,
      inboxFolder(m.folder),
      +m.unread,
      +m.starred,
      m.timestamp,
      JSON.stringify(m),
      `${senderName(m.sender)} ${m.subject} ${mail.text}`.toLowerCase(),
    ]).stepReset();
  }
  private async synchronize(
    tx: IDBTransaction,
    state: CacheState & { epoch: string },
  ) {
    const checkpoint = this.sql!.selectObject(
      "SELECT epoch,revision FROM checkpoint WHERE singleton=1",
    ) as { epoch: string; revision: number } | undefined;
    const same = checkpoint?.epoch === state.epoch;
    let incremental = same && checkpoint.revision === state.revision;
    if (
      same &&
      checkpoint.revision >= state.floor &&
      checkpoint.revision < state.revision &&
      state.revision - checkpoint.revision <= 1024
    ) {
      const changes = await read<CacheChange[]>(
        tx
          .objectStore("mailChanges")
          .getAll(
            IDBKeyRange.bound(checkpoint.revision, state.revision, true),
            1024,
          ),
      );
      incremental =
        changes.length === state.revision - checkpoint.revision &&
        changes.every(
          (c, i) =>
            c.revision === checkpoint.revision + i + 1 &&
            !c.reset &&
            typeof c.id === "string",
        );
      if (incremental)
        for (const id of new Set(changes.map((c) => c.id!))) {
          this.put(
            id,
            await read<RecordMail | undefined>(tx.objectStore("mail").get(id)),
          );
          const alias = await read<MailAlias | undefined>(
            tx.objectStore("mailAliases").get(id),
          );
          if (alias)
            this.exec(
              "INSERT INTO aliases VALUES(?,?) ON CONFLICT(alias) DO UPDATE SET target=excluded.target",
              [id, alias.target],
            );
          else this.exec("DELETE FROM aliases WHERE alias=?", [id]);
        }
    }
    if (!incremental) {
      // Rebuild FTS once from the completed table instead of tokenizing/merging
      // after every cursor row. DDL and content remain in this transaction;
      // a failed read restores both the old data and its triggers/indexes.
      this.exec(
        "DROP TRIGGER messages_insert; DROP TRIGGER messages_delete; DROP TRIGGER messages_update; DELETE FROM messages; DELETE FROM aliases;",
      );
      await walk(tx.objectStore("mail").openCursor(), (row) =>
        this.put(String(row.primaryKey), row.value),
      );
      await walk(tx.objectStore("mailAliases").openCursor(), (row) =>
        this.exec("INSERT INTO aliases VALUES(?,?)", [
          String(row.primaryKey),
          (row.value as MailAlias).target,
        ]),
      );
    }
    if (!incremental) {
      this.exec("INSERT INTO terms(terms) VALUES('rebuild')");
      this.exec(triggers);
    }
    // Account settings can change without changing a message revision.
    this.exec(
      "DELETE FROM messages WHERE account NOT IN(SELECT id FROM accounts)",
    );
    if (!same || checkpoint.revision !== state.revision)
      this.exec(
        "INSERT INTO checkpoint VALUES(1,?,?) ON CONFLICT(singleton) DO UPDATE SET epoch=excluded.epoch,revision=excluded.revision",
        [state.epoch, state.revision],
      );
  }
  async page(input: MailboxQuery, started?: () => void): Promise<MailboxPage> {
    const query = checkedQuery(input);
    if (this.closed) throw Error("Mailbox storage is closed. Reopen Shep.");
    return navigator.locks.request(
      `shep.${this.user}.mailbox-index`,
      async () => {
        if (this.closed) throw Error("Mailbox storage is closed. Reopen Shep.");
        try {
          if (!this.pool)
            this.pool = await this.sqlite.installOpfsSAHPoolVfs({
              name: `shep-mailbox-${this.user}`,
              directory: `/shep-mailbox-v1/${this.user}`,
              initialCapacity: 4,
            });
          else await this.pool.unpauseVfs();
          this.sql = new this.pool.OpfsSAHPoolDb("/mailbox.sqlite");
          this.exec(schema);
          this.exec(triggers);
          this.insert = this.sql.prepare(insertMail);
          const bulk = new BulkProjection(this.sql, this.user);
          this.exec("BEGIN");
          try {
            const state = await snapshot(
              this.cache,
              sourceStores,
              async (tx) => {
                const state = sourceState(
                  await read<CacheState | undefined>(
                    tx.objectStore("cacheState").get("mail"),
                  ),
                );
                const accounts = await read<Account[]>(
                  tx.objectStore("accounts").getAll(),
                );
                const roles = await read<MailRoles[]>(
                  tx.objectStore("mailRoles").getAll(),
                );
                if (
                  query.scope.account &&
                  !accounts.some((a) => a.id === query.scope.account)
                )
                  throw Error(
                    "This account was removed. Open a connected account.",
                  );
                this.accountSet = new Set(accounts.map((a) => a.id));
                for (const account of accounts) {
                  this.exec("INSERT INTO accounts VALUES(?,?)", [
                    account.id,
                    account.email,
                  ]);
                  for (const folder of roleFolders(
                    account,
                    roles.find((r) => r.account === account.id),
                  ))
                    this.exec("INSERT OR IGNORE INTO sent VALUES(?,?)", [
                      account.id,
                      inboxFolder(folder),
                    ]);
                }
                started?.();
                await this.synchronize(tx, state);
                await bulk.source(tx, state, accounts);
                return state;
              },
            );
            // Release the mail snapshot before derived queries. Receipt writes
            // precede cache writes, so observe the journal after the source:
            // an acknowledged cache row can never lack its before receipt.
            const bulkRevision = await bulk.journal();
            const result = (() => {
              bulk.materialize();
              for (const [id, fields] of Object.entries(
                query.scope.projection ?? {},
              )) {
                this.exec(
                  "INSERT INTO pending VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET folder=COALESCE(excluded.folder,pending.folder),unread=COALESCE(excluded.unread,pending.unread),starred=COALESCE(excluded.starred,pending.starred)",
                  [
                    this.canonical(id),
                    fields.folder === undefined
                      ? null
                      : inboxFolder(fields.folder),
                    fields.unread === undefined ? null : +fields.unread,
                    fields.starred === undefined ? null : +fields.starred,
                  ],
                );
              }
              const where: string[] = [],
                bind: Bind = [],
                folder = inboxFolder(query.scope.folder);
              const effective = `WITH effective AS (SELECT m.rowid,m.id,m.account,COALESCE(p.folder,b.folder,m.folder) folder,COALESCE(p.unread,b.unread,m.unread) unread,COALESCE(p.starred,b.starred,m.starred) starred,m.timestamp,m.core,m.search FROM messages m LEFT JOIN pending p ON p.id=m.id LEFT JOIN bulk_current b ON b.id=m.id)`;
              where.push(
                folder === "Sent"
                  ? "(e.folder=? OR EXISTS(SELECT 1 FROM sent WHERE account=e.account AND folder=e.folder))"
                  : "e.folder=?",
              );
              bind.push(folder);
              if (query.scope.account) {
                where.push("e.account=?");
                bind.push(query.scope.account);
              }
              if (query.scope.filter === "Unread") where.push("e.unread=1");
              if (query.scope.filter === "Flagged") where.push("e.starred=1");
              const words = (query.scope.query ?? "")
                .toLowerCase()
                .trim()
                .split(/\s+/)
                .filter(Boolean);
              const indexed = words.filter(
                (w) => [...w].length >= 3 && !w.includes("\0"),
              );
              if (indexed.length) {
                where.push(
                  "e.rowid IN(SELECT rowid FROM terms WHERE terms MATCH ?)",
                );
                bind.push(
                  indexed
                    .map((w) => `"${w.replaceAll('"', '""')}"`)
                    .join(" AND "),
                );
              }
              for (const word of words) {
                where.push("instr(e.search,?)>0");
                bind.push(word);
              }
              const from = `FROM effective e JOIN accounts a ON a.id=e.account WHERE ${where.join(" AND ")}`;
              const observedBefore = query.undo
                ? (this.sql!.selectValues(
                    `${effective} SELECT e.id ${from} ORDER BY e.timestamp ${query.scope.oldest ? "ASC" : "DESC"},e.id ASC LIMIT 50 OFFSET ?`,
                    [...bind, query.offset],
                  ) as string[])
                : [];
              const undo = query.undo
                ? bulk.previewUndo(query.undo)
                : undefined;
              try {
                const total = this.value(
                  `${effective} SELECT COUNT(*) ${from}`,
                  bind,
                );
                const unread = this.value(
                  `${effective} SELECT COUNT(*) FROM effective WHERE folder='INBOX' AND unread=1`,
                );
                const confirmed: Record<string, import("./model").Fields> =
                  Object.create(null);
                const rows = this.sql!.selectObjects(
                  `${effective} SELECT e.id,e.core,e.folder,e.unread,e.starred,a.email ${from} ORDER BY e.timestamp ${query.scope.oldest ? "ASC" : "DESC"},e.id ASC LIMIT 50 OFFSET ?`,
                  [...bind, query.offset],
                ).map((row) => {
                  const core = JSON.parse(row.core as string) as CoreMail;
                  confirmed[row.id as string] = {
                    folder:
                      inboxFolder(core.folder) === "INBOX"
                        ? "Inbox"
                        : core.folder,
                    unread: core.unread,
                    starred: core.starred,
                  };
                  return display(
                    {
                      ...core,
                      folder: row.folder as string,
                      unread: !!row.unread,
                      starred: !!row.starred,
                    },
                    row.id as string,
                    row.email as string,
                  );
                });
                const aliases: Record<string, string> = Object.create(null);
                for (const id of [
                  ...(query.observed ?? []),
                  ...Object.keys(query.scope.projection ?? {}),
                ]) {
                  const target = this.canonical(id);
                  if (id !== target) aliases[id] = target;
                }
                const result: MailboxPage = {
                  epoch: state.epoch,
                  revision: state.revision,
                  bulkRevision,
                  groupFields: Object.fromEntries(
                    [
                      ...new Set([
                        ...rows.map((row) => row.id),
                        ...(query.observed ?? []).map((id) =>
                          this.canonical(id),
                        ),
                      ]),
                    ].map((id) => {
                      const fields = bulk.fields(id);
                      return [
                        id,
                        {
                          ...fields,
                          ...(fields.folder === "INBOX"
                            ? { folder: "Inbox" }
                            : {}),
                        },
                      ];
                    }),
                  ),
                  total,
                  unread,
                  rows,
                  aliases,
                  confirmed,
                };
                if (undo) {
                  const ids = new Set([
                    ...observedBefore,
                    ...rows.map((r) => r.id),
                    ...(query.observed ?? []).map((id) => this.canonical(id)),
                  ]);
                  const textMatches: Record<string, boolean> = {};
                  result.groupFields = {};
                  const keys = [...ids],
                    placeholders = keys.map(() => "?").join(",");
                  const predicted = keys.length
                    ? this.sql!.selectObjects(
                        `${effective} SELECT id,folder,unread,starred,${words.map(() => "instr(search,?)>0").join(" AND ") || "1"} text_matches FROM effective WHERE id IN(${placeholders})`,
                        [...words, ...keys],
                      )
                    : [];
                  for (const row of predicted) {
                    const id = row.id as string;
                    result.groupFields[id] = {
                      folder:
                        row.folder === "INBOX"
                          ? "Inbox"
                          : (row.folder as string),
                      unread: !!row.unread,
                      starred: !!row.starred,
                    };
                    textMatches[id] = !!row.text_matches;
                  }
                  undo.restore();
                  const beforeFields: Record<string, import("./model").Fields> =
                    {};
                  const previous = keys.length
                    ? this.sql!.selectObjects(
                        `${effective} SELECT id,folder,unread,starred FROM effective WHERE id IN(${placeholders})`,
                        keys,
                      )
                    : [];
                  for (const row of previous)
                    beforeFields[row.id as string] = {
                      folder:
                        row.folder === "INBOX"
                          ? "Inbox"
                          : (row.folder as string),
                      unread: !!row.unread,
                      starred: !!row.starred,
                    };
                  result.undo = {
                    id: undo.id,
                    revision: undo.revision,
                    committed: undo.committed,
                    textMatches,
                    beforeFields,
                  };
                }
                return result;
              } finally {
                undo?.restore();
              }
            })();
            this.exec("COMMIT");
            return result;
          } catch (error) {
            if (this.sql.isOpen()) this.exec("ROLLBACK");
            throw error;
          }
        } finally {
          this.insert?.finalize();
          this.insert = undefined;
          this.sql?.close();
          this.sql = undefined;
          this.pool?.pauseVfs();
        }
      },
    );
  }
}

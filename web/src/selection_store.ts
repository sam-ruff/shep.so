import sqliteInit, {
  type Database,
  type Sqlite3Static,
} from "@sqlite.org/sqlite-wasm";
import sqliteWasm from "@sqlite.org/sqlite-wasm/sqlite3.wasm?url";
import { openMailDatabase } from "./storage";
import { mailMatches, senderName } from "./mail_query";
import { roleFolders, type MailAlias, type MailRoles } from "./sent_cache";
import type { Account, RecordMail } from "./provider";
import type { CacheMail, CacheState, CacheChange } from "./cache_changes";
import {
  selectionScope,
  selectionToken,
  type SelectionScope,
  type SelectionCommand,
  type SelectionResult,
  type SelectionSnapshot,
  type SelectionPage,
  type SelectionGroup,
} from "./selection_types";

type Bind = (string | number | null)[];
interface Session {
  id: string;
  revision: number;
  frozen: number;
  scope: string;
}
function read<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}
function walk(
  request: IDBRequest<IDBCursorWithValue | null>,
  visit: (cursor: IDBCursorWithValue) => void | Promise<void>,
): Promise<void> {
  return new Promise((resolve, reject) => {
    request.onerror = () => reject(request.error);
    request.onsuccess = async () => {
      const cursor = request.result;
      if (!cursor) {
        resolve();
        return;
      }
      try {
        await visit(cursor);
        cursor.continue();
      } catch (error) {
        reject(error);
      }
    };
  });
}
function readonly<T>(
  cache: IDBDatabase,
  names: string[],
  operation: (tx: IDBTransaction) => Promise<T>,
): Promise<T> {
  return new Promise((resolve, reject) => {
    const tx = cache.transaction(names, "readonly");
    let result: T, cause: unknown;
    tx.oncomplete = () => resolve(result);
    tx.onabort = () =>
      reject(
        cause ?? Error("Could not read cached mail. Retry the selection."),
      );
    // Inside this transaction, await IDB requests only. SQLite operations are
    // synchronous in this worker; they do not deactivate the IDB snapshot.
    void operation(tx).then(
      (value) => {
        result = value;
      },
      (error) => {
        cause = error;
        try {
          tx.abort();
        } catch {
          reject(error);
        }
      },
    );
  });
}
function revision(value: number) {
  if (!Number.isSafeInteger(value) || value < 0)
    throw Error("Invalid selection revision. Select messages again.");
}
const schema = `PRAGMA foreign_keys=ON;
 CREATE TABLE sessions(id TEXT PRIMARY KEY,revision INTEGER NOT NULL,frozen INTEGER NOT NULL,scope TEXT NOT NULL);
 CREATE TABLE membership(token TEXT REFERENCES sessions(id) ON DELETE CASCADE,id TEXT NOT NULL,position INTEGER NOT NULL,selected INTEGER NOT NULL,PRIMARY KEY(token,id),UNIQUE(token,position));
 CREATE INDEX chosen ON membership(token,selected,position);
 CREATE INDEX member_id ON membership(id);
 CREATE TABLE meta(id TEXT PRIMARY KEY,account TEXT NOT NULL,folder TEXT NOT NULL,unread INTEGER NOT NULL,starred INTEGER NOT NULL,present INTEGER NOT NULL);
 CREATE TABLE aliases(alias TEXT PRIMARY KEY,target TEXT NOT NULL);
 CREATE TEMP TABLE handover(token TEXT,position INTEGER,selected INTEGER);`;

/** Temporary membership belongs to this worker's SQLite connection. The mail
 * cache is opened read-only, so a large capture cannot own its write lock. */
export class SelectionStore {
  private lastRevision = -1;
  private constructor(
    private cache: IDBDatabase,
    private sql: Database,
  ) {
    sql.exec(schema);
  }
  static async open(user: string) {
    const cache = await openMailDatabase(user);
    try {
      // The pinned Emscripten module accepts locateFile although the npm
      // wrapper's declaration omits its optional moduleArg (see dist/index.mjs).
      const initialize = sqliteInit as unknown as (options: {
        locateFile: (name: string) => string;
      }) => Promise<Sqlite3Static>;
      const sqlite = await initialize({ locateFile: () => sqliteWasm });
      return new SelectionStore(cache, new sqlite.oo1.DB(":memory:"));
    } catch (error) {
      cache.close();
      throw error;
    }
  }
  close() {
    this.sql.close();
    this.cache.close();
  }
  private exec(sql: string, bind: Bind = []) {
    this.sql.exec({ sql, bind: bind.length ? bind : undefined });
  }
  private row<T>(sql: string, bind: Bind = []): T | undefined {
    return this.sql.selectObject(sql, bind.length ? bind : undefined) as
      | T
      | undefined;
  }
  private rows<T>(sql: string, bind: Bind = []): T[] {
    return this.sql.selectObjects(sql, bind.length ? bind : undefined) as T[];
  }
  private value(sql: string, bind: Bind = []) {
    return Number(
      this.sql.selectValue(sql, bind.length ? bind : undefined) ?? 0,
    );
  }
  private session(id: string): Session {
    const result = this.row<Session>("SELECT * FROM sessions WHERE id=?", [id]);
    if (!result)
      throw Error(
        "This selection is no longer available. Select the messages again.",
      );
    return result;
  }
  private putMeta(mail: CacheMail) {
    const m = mail.core;
    this.exec(
      "INSERT INTO meta VALUES(?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET account=excluded.account,folder=excluded.folder,unread=excluded.unread,starred=excluded.starred,present=excluded.present",
      [mail.id, m.account_id, m.folder, +m.unread, +m.starred, +!mail.moved],
    );
  }
  private adopt(old: string, target: string) {
    if (old === target) return;
    this.exec("DELETE FROM handover");
    this.exec(
      "INSERT INTO handover SELECT token,position,selected FROM membership WHERE id=?",
      [old],
    );
    this.exec("DELETE FROM membership WHERE id=?", [old]);
    this.exec(
      `INSERT INTO membership SELECT token,?,position,selected FROM handover WHERE true
      ON CONFLICT(token,id) DO UPDATE SET position=MIN(membership.position,excluded.position),selected=MAX(membership.selected,excluded.selected)`,
      [target],
    );
    // A temporarily missing target still belongs to its original account.
    // Preserve that ownership for missing counts and explicit account removal.
    this.exec(
      "INSERT INTO meta SELECT ?,account,folder,unread,starred,0 FROM meta WHERE id=? ON CONFLICT(id) DO NOTHING",
      [target, old],
    );
    this.exec("DELETE FROM meta WHERE id=?", [old]);
    this.exec(
      "INSERT INTO aliases VALUES(?,?) ON CONFLICT(alias) DO UPDATE SET target=excluded.target",
      [old, target],
    );
    this.exec("UPDATE aliases SET target=? WHERE target=?", [target, old]);
  }
  private async canonical(tx: IDBTransaction, id: string) {
    const alias = await read<MailAlias | undefined>(
      tx.objectStore("mailAliases").get(id),
    );
    if (alias) {
      this.adopt(id, alias.target);
      return alias.target;
    }
    this.exec("DELETE FROM aliases WHERE alias=?", [id]);
    return id;
  }
  private async updateMeta(tx: IDBTransaction, id: string) {
    const target = await this.canonical(tx, id);
    if (
      !this.value("SELECT EXISTS(SELECT 1 FROM membership WHERE id=?)", [
        target,
      ])
    )
      return;
    const mail = await read<CacheMail | undefined>(
      tx.objectStore("mailMetadata").get(target),
    );
    if (mail) this.putMeta(mail);
    else this.exec("UPDATE meta SET present=0 WHERE id=?", [target]);
  }
  private async synchronize(
    tx: IDBTransaction,
    state: CacheState,
    accounts: Account[],
  ) {
    // Explicit account removal removes only its captured membership. A missing
    // individual message remains counted as selected but unavailable.
    const ids = JSON.stringify(accounts.map((a) => a.id));
    this.exec(
      "DELETE FROM membership WHERE id IN(SELECT id FROM meta WHERE account NOT IN(SELECT value FROM json_each(?)))",
      [ids],
    );
    this.exec(
      "DELETE FROM meta WHERE account NOT IN(SELECT value FROM json_each(?))",
      [ids],
    );
    if (
      state.revision === this.lastRevision ||
      !this.value("SELECT EXISTS(SELECT 1 FROM membership)")
    )
      return;
    let complete = false;
    if (this.lastRevision >= state.floor && this.lastRevision >= 0) {
      const changes = await read<CacheChange[]>(
        tx
          .objectStore("mailChanges")
          .getAll(IDBKeyRange.bound(this.lastRevision, state.revision, true)),
      );
      complete =
        changes.length === state.revision - this.lastRevision &&
        changes.length <= 1024 &&
        changes.every(
          (c, i) =>
            c.revision === this.lastRevision + i + 1 &&
            !c.reset &&
            typeof c.id === "string",
        );
      if (complete)
        for (const change of changes) await this.updateMeta(tx, change.id!);
    }
    if (complete) return;
    // A sleeping worker can outlive the bounded replay window. Rebuild current
    // metadata from one read snapshot without changing captured membership.
    this.exec("UPDATE meta SET present=0");
    this.exec("DELETE FROM aliases");
    const update = this.sql.prepare(
      "UPDATE meta SET account=?,folder=?,unread=?,starred=?,present=? WHERE id=?",
    );
    try {
      await walk(tx.objectStore("mailMetadata").openCursor(), (cursor) => {
        const mail = cursor.value as CacheMail,
          m = mail.core;
        update
          .bind([
            m.account_id,
            m.folder,
            +m.unread,
            +m.starred,
            +!mail.moved,
            mail.id,
          ])
          .stepReset();
      });
    } finally {
      update.finalize();
    }
    await walk(tx.objectStore("mailAliases").openCursor(), async (cursor) => {
      const alias = cursor.value as MailAlias;
      if (
        this.value("SELECT EXISTS(SELECT 1 FROM membership WHERE id=?)", [
          alias.alias,
        ])
      )
        await this.updateMeta(tx, alias.alias);
    });
  }
  private async context(
    tx: IDBTransaction,
    scope: SelectionScope,
    accounts: Account[],
  ) {
    if (scope.account && !accounts.some((a) => a.id === scope.account))
      throw Error(
        "This account was removed. Select messages from a connected account.",
      );
    const roles = new Map(
      (await read<MailRoles[]>(tx.objectStore("mailRoles").getAll())).map(
        (r) => [r.account, r],
      ),
    );
    const folders = new Map(
      accounts.map((a) => [a.id, roleFolders(a, roles.get(a.id))]),
    );
    const projection = new Map<string, Record<string, unknown>>();
    for (const [id, fields] of Object.entries(scope.projection ?? {})) {
      const target = await this.canonical(tx, id);
      if (projection.has(target))
        throw Error(
          "This message identity changed. Refresh the folder and retry.",
        );
      projection.set(target, fields);
    }
    return async (mail: CacheMail) => {
      const m = mail.core;
      if (
        mail.moved ||
        !folders.has(m.account_id) ||
        (scope.account && m.account_id !== scope.account)
      )
        return false;
      const fields = projection.get(mail.id);
      // Only search needs the cached body. Ordinary captures read metadata.
      const body = scope.query?.trim()
        ? ((
            await read<RecordMail | undefined>(
              tx.objectStore("mail").get(mail.id),
            )
          )?.text ?? "")
        : "";
      return !!mailMatches(
        {
          ...m,
          sender: senderName(m.sender),
          body,
          folder: typeof fields?.folder === "string" ? fields.folder : m.folder,
          unread:
            typeof fields?.unread === "boolean" ? fields.unread : m.unread,
          starred:
            typeof fields?.starred === "boolean" ? fields.starred : m.starred,
        },
        scope,
        folders.get(m.account_id),
      );
    };
  }
  private async snapshot(
    tx: IDBTransaction,
    id: string,
    observed: string[],
  ): Promise<SelectionSnapshot> {
    const visible = new Set<string>(),
      positions: Record<string, number> = {},
      aliases: Record<string, string> = {};
    for (const original of observed) {
      const target = await this.canonical(tx, original);
      if (target !== original) {
        aliases[original] = target;
        await this.updateMeta(tx, target);
      }
      const row = this.row<{
        position: number;
        selected: number;
        present: number;
      }>(
        "SELECT s.position,s.selected,COALESCE(m.present,0) AS present FROM membership s LEFT JOIN meta m ON m.id=s.id WHERE s.token=? AND s.id=?",
        [id, target],
      );
      if (row) {
        positions[target] = row.position;
        if (row.selected && row.present) visible.add(target);
      }
    }
    const s = this.session(id),
      counts = this.row<{ total: number; selected: number }>(
        "SELECT COUNT(*) AS total,COALESCE(SUM(selected),0) AS selected FROM membership WHERE token=?",
        [id],
      )!;
    const available = this.row<{
      available: number;
      unread: number;
      starred: number;
    }>(
      "SELECT COUNT(*) AS available,COALESCE(SUM(m.unread),0) AS unread,COALESCE(SUM(m.starred),0) AS starred FROM membership s JOIN meta m ON s.id=m.id WHERE s.token=? AND s.selected=1 AND m.present=1",
      [id],
    )!;
    const groups = this.rows<SelectionGroup>(
      "SELECT m.account,m.folder,COUNT(*) AS total,SUM(m.unread) AS unread,SUM(m.starred) AS starred FROM membership s JOIN meta m ON s.id=m.id WHERE s.token=? AND s.selected=1 AND m.present=1 GROUP BY m.account,m.folder ORDER BY m.account,m.folder",
      [id],
    );
    return {
      id,
      revision: s.revision,
      frozen: !!s.frozen,
      ...counts,
      ...available,
      groups,
      visible: [...visible].sort(),
      positions,
      aliases,
    };
  }
  private prune() {
    this.exec("DELETE FROM meta WHERE id NOT IN(SELECT id FROM membership)");
    this.exec(
      "DELETE FROM aliases WHERE target NOT IN(SELECT id FROM membership)",
    );
  }
  async run(
    command: SelectionCommand,
    observed: string[] = [],
    snapshotStarted: () => void = () => {},
  ): Promise<SelectionResult> {
    if (observed.length > 50) throw Error("Observe one mail page at a time.");
    selectionToken(command.id);
    this.exec("BEGIN");
    try {
      if (command.kind === "release") {
        this.exec("DELETE FROM sessions WHERE id=?", [command.id]);
        this.prune();
        this.exec("COMMIT");
        return null;
      }
      const names = [
        "mailMetadata",
        "cacheState",
        "mailChanges",
        "accounts",
        "mailAliases",
        "mailRoles",
      ];
      if ("scope" in command && command.scope.query?.trim()) names.push("mail");
      let nextRevision = this.lastRevision;
      const result = await readonly(this.cache, names, async (tx) => {
        const state = await read<CacheState>(
          tx.objectStore("cacheState").get("mail"),
        );
        if (
          !state ||
          !Number.isSafeInteger(state.revision) ||
          state.revision < this.lastRevision
        )
          throw Error(
            "The mail cache changed. Choose Done and select the messages again.",
          );
        const accounts = await read<Account[]>(
          tx.objectStore("accounts").getAll(),
        );
        snapshotStarted();
        await this.synchronize(tx, state, accounts);
        nextRevision = state.revision;
        if (command.kind === "capture") {
          revision(command.revision);
          const old = this.row<Session>("SELECT * FROM sessions WHERE id=?", [
            command.id,
          ]);
          if (old && (old.frozen || old.revision >= command.revision))
            throw Error("The selection changed. Select the messages again.");
          const matches = await this.context(tx, command.scope, accounts);
          this.exec("DELETE FROM sessions WHERE id=?", [command.id]);
          this.exec("INSERT INTO sessions VALUES(?,?,0,?)", [
            command.id,
            command.revision,
            JSON.stringify(selectionScope(command.scope)),
          ]);
          const put = this.sql.prepare(
            "INSERT INTO meta VALUES(?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET account=excluded.account,folder=excluded.folder,unread=excluded.unread,starred=excluded.starred,present=excluded.present",
          );
          const insert = this.sql.prepare(
            "INSERT INTO membership VALUES(?,?,?,?)",
          );
          let position = 0;
          try {
            await walk(
              tx
                .objectStore("mailMetadata")
                .index(command.scope.oldest ? "oldest" : "newest")
                .openCursor(),
              async (cursor) => {
                const mail = cursor.value as CacheMail;
                if (!(await matches(mail))) return;
                const m = mail.core;
                put
                  .bind([
                    mail.id,
                    m.account_id,
                    m.folder,
                    +m.unread,
                    +m.starred,
                    +!mail.moved,
                  ])
                  .stepReset();
                insert
                  .bind([command.id, mail.id, position++, +command.all])
                  .stepReset();
              },
            );
          } finally {
            put.finalize();
            insert.finalize();
          }
          this.prune();
        } else {
          const s = this.session(command.id);
          if ("expected" in command) {
            revision(command.expected);
            if (command.expected !== s.revision)
              throw Error("The selection changed. Review or select it again.");
          }
          if (command.kind === "change") {
            if (s.frozen || s.revision === Number.MAX_SAFE_INTEGER)
              throw Error("This review is frozen. Select the messages again.");
            if (s.scope !== JSON.stringify(selectionScope(command.scope)))
              throw Error(
                "The mailbox view changed. Select its messages again.",
              );
            const c = command.change;
            if (c.kind === "clear")
              this.exec("UPDATE membership SET selected=0 WHERE token=?", [
                s.id,
              ]);
            else if (c.kind === "set") {
              const id = await this.canonical(tx, c.id);
              if (
                !this.value(
                  "SELECT EXISTS(SELECT 1 FROM membership WHERE token=? AND id=?)",
                  [s.id, id],
                )
              ) {
                const matches = await this.context(tx, command.scope, accounts),
                  mail = await read<CacheMail | undefined>(
                    tx.objectStore("mailMetadata").get(id),
                  );
                if (!mail || !(await matches(mail)))
                  throw Error(
                    "This message is outside the current mailbox view.",
                  );
                this.putMeta(mail);
                this.exec(
                  "INSERT INTO membership SELECT ?1,?2,COALESCE(MAX(position),-1)+1,0 FROM membership WHERE token=?1",
                  [s.id, id],
                );
              }
              if (c.clear_others)
                this.exec("UPDATE membership SET selected=0 WHERE token=?", [
                  s.id,
                ]);
              this.exec(
                "UPDATE membership SET selected=? WHERE token=? AND id=?",
                [+c.selected, s.id, id],
              );
            } else if (c.kind === "range") {
              const a = await this.canonical(tx, c.anchor),
                b = await this.canonical(tx, c.target);
              const from = this.row<{ position: number }>(
                "SELECT position FROM membership WHERE token=? AND id=?",
                [s.id, a],
              );
              const to = this.row<{ position: number }>(
                "SELECT position FROM membership WHERE token=? AND id=?",
                [s.id, b],
              );
              if (!from || !to)
                throw Error(
                  "This message is outside the captured selection. Select all again to include new arrivals.",
                );
              if (!c.additive)
                this.exec("UPDATE membership SET selected=0 WHERE token=?", [
                  s.id,
                ]);
              this.exec(
                "UPDATE membership SET selected=1 WHERE token=? AND position BETWEEN ? AND ?",
                [
                  s.id,
                  Math.min(from.position, to.position),
                  Math.max(from.position, to.position),
                ],
              );
            } else throw Error("Unsupported selection action.");
            this.exec("UPDATE sessions SET revision=revision+1 WHERE id=?", [
              s.id,
            ]);
          } else if (command.kind === "freeze") {
            selectionToken(command.target);
            this.exec("INSERT INTO sessions VALUES(?,0,1,?)", [
              command.target,
              s.scope,
            ]);
            this.exec(
              "INSERT INTO membership SELECT ?,id,position,1 FROM membership WHERE token=? AND selected=1",
              [command.target, s.id],
            );
            return this.snapshot(tx, command.target, observed);
          } else if (command.kind === "page") {
            const after = command.after ?? -1;
            if (after !== -1) revision(after);
            const rows = this.rows<{
              position: number;
              id: string;
              account: string;
              folder: string;
              unread: number;
              starred: number;
            }>(
              "SELECT s.position,s.id,m.account,m.folder,m.unread,m.starred FROM membership s JOIN meta m ON m.id=s.id WHERE s.token=? AND s.selected=1 AND m.present=1 AND s.position>? ORDER BY s.position LIMIT 50",
              [s.id, after],
            );
            return {
              revision: s.revision,
              rows: rows.map((r) => ({
                ...r,
                unread: !!r.unread,
                starred: !!r.starred,
              })),
              next_after: rows.length === 50 ? rows.at(-1)!.position : null,
            } satisfies SelectionPage;
          } else if (command.kind !== "observe")
            throw Error("Unsupported selection action.");
        }
        return this.snapshot(tx, command.id, observed);
      });
      this.exec("COMMIT");
      this.lastRevision = nextRevision;
      return result;
    } catch (error) {
      this.exec("ROLLBACK");
      throw error;
    }
  }
}

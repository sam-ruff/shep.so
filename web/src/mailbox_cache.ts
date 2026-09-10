import { openMailDatabase } from "./storage";
import { inboxFolder, senderName } from "./mail_query";
import type { Account, CoreMail, RecordMail } from "./provider";
import type { CacheState, CacheMail } from "./cache_changes";
import type { MailAlias } from "./sent_cache";
import type { Mail } from "./model";
import type {
  MailboxDetail,
  MailboxMetadata,
  MailScanQuery,
  MailScanPage,
  MailScanEntry,
} from "./mailbox_types";
export function read<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}
export function walk(
  request: IDBRequest<IDBCursorWithValue | null>,
  visit: (cursor: IDBCursorWithValue) => void,
): Promise<void> {
  return new Promise((resolve, reject) => {
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      const cursor = request.result;
      if (!cursor) return resolve();
      try {
        visit(cursor);
        cursor.continue();
      } catch (error) {
        reject(error);
      }
    };
  });
}
export function snapshot<T>(
  cache: IDBDatabase,
  names: string[],
  operation: (tx: IDBTransaction) => Promise<T>,
): Promise<T> {
  return new Promise((resolve, reject) => {
    const tx = cache.transaction(names, "readonly");
    let result: T, cause: unknown;
    tx.oncomplete = () => resolve(result);
    tx.onabort = () =>
      reject(cause ?? Error("Could not read cached mail. Retry Refresh."));
    // Await only IndexedDB requests inside the snapshot; synchronous SQLite
    // operations stay in this worker and cannot deactivate the source view.
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
export function sourceState(
  value: CacheState | undefined,
): CacheState & { epoch: string } {
  if (
    !value ||
    typeof value.epoch !== "string" ||
    !value.epoch ||
    !Number.isSafeInteger(value.revision) ||
    value.revision < 0 ||
    !Number.isSafeInteger(value.floor) ||
    value.floor < 0 ||
    value.floor > value.revision
  )
    throw Error(
      "The cached mailbox revision is unavailable. Reopen Shep to retry.",
    );
  return value as CacheState & { epoch: string };
}
export function display(core: CoreMail, id: string, email: string): Mail {
  return {
    id,
    accountId: core.account_id,
    account: email,
    sender: senderName(core.sender),
    address: core.sender.match(/<([^<>]+)>/)?.[1] ?? core.sender,
    subject: core.subject,
    preview: core.preview,
    body: "",
    bodyLoaded: false,
    folder: inboxFolder(core.folder) === "INBOX" ? "Inbox" : core.folder,
    date: new Date(core.timestamp * 1000).toISOString(),
    unread: core.unread,
    starred: core.starred,
    attachments: Array.from(
      { length: Math.min(core.attachment_count, 100) },
      (_, i) => `Attachment ${i + 1}`,
    ),
  };
}

export class MailboxReads {
  private constructor(private cache: IDBDatabase) {}
  static async open(user: string) {
    return new MailboxReads(await openMailDatabase(user));
  }
  close() {
    this.cache.close();
  }
  async metadata(id: string): Promise<MailboxMetadata> {
    if (typeof id !== "string" || !id) throw Error("Choose a message to read.");
    return snapshot(
      this.cache,
      ["mailMetadata", "mailAliases", "accounts", "cacheState"],
      async (tx) => {
        const alias = await read<MailAlias | undefined>(
          tx.objectStore("mailAliases").get(id),
        );
        const target = alias?.target ?? id;
        const value = await read<CacheMail | undefined>(
          tx.objectStore("mailMetadata").get(target),
        );
        const account =
          value &&
          (await read<Account | undefined>(
            tx.objectStore("accounts").get(value.core.account_id),
          ));
        const state = sourceState(
          await read<CacheState | undefined>(
            tx.objectStore("cacheState").get("mail"),
          ),
        );
        return {
          id: target,
          epoch: state.epoch,
          revision: state.revision,
          mail:
            value && !value.moved && account
              ? display(value.core, target, account.email)
              : undefined,
        };
      },
    );
  }
  async detail(id: string): Promise<MailboxDetail> {
    if (typeof id !== "string" || !id) throw Error("Choose a message to read.");
    return snapshot(
      this.cache,
      ["mail", "mailAliases", "accounts", "cacheState"],
      async (tx) => {
        const alias = await read<MailAlias | undefined>(
          tx.objectStore("mailAliases").get(id),
        );
        const target = alias?.target ?? id;
        const mail = await read<RecordMail | undefined>(
          tx.objectStore("mail").get(target),
        );
        if (
          !mail ||
          mail.moved ||
          !(await read(tx.objectStore("accounts").get(mail.core.account_id)))
        )
          throw Error(
            "This message is no longer cached. Refresh its account and choose it again.",
          );
        if (typeof mail.text !== "string")
          throw Error(
            "The cached message body is damaged. Refresh its account and retry.",
          );
        const state = sourceState(
          await read<CacheState | undefined>(
            tx.objectStore("cacheState").get("mail"),
          ),
        );
        return {
          id: target,
          body: mail.text,
          revision: state.revision,
          epoch: state.epoch,
        };
      },
    );
  }
  async scan(query: MailScanQuery): Promise<MailScanPage> {
    if (
      typeof query?.account !== "string" ||
      !query.account ||
      [query.folder, query.serverId, query.after].some(
        (v) => v != null && typeof v !== "string",
      )
    )
      throw Error("Invalid mailbox scan. Refresh the account.");
    return snapshot(this.cache, ["mail", "accounts"], async (tx) => {
      if (!(await read(tx.objectStore("accounts").get(query.account))))
        return { rows: [], next: null };
      const index =
        query.serverId !== undefined
          ? "serverIdentity"
          : query.folder !== undefined
            ? "accountFolder"
            : "account";
      const key =
        query.serverId !== undefined
          ? [query.account, query.serverId]
          : query.folder !== undefined
            ? [query.account, query.folder]
            : query.account;
      const rows: MailScanEntry[] = [];
      return new Promise<MailScanPage>((resolve, reject) => {
        const request = tx
          .objectStore("mail")
          .index(index)
          .openCursor(IDBKeyRange.only(key));
        request.onerror = () => reject(request.error);
        request.onsuccess = () => {
          const cursor = request.result;
          if (!cursor) {
            resolve({ rows, next: null });
            return;
          }
          if (query.after != null) {
            const compared = indexedDB.cmp(cursor.primaryKey, query.after);
            if (compared < 0) {
              cursor.continuePrimaryKey(key, query.after);
              return;
            }
            if (compared === 0) {
              cursor.continue();
              return;
            }
          }
          const mail = cursor.value as RecordMail;
          rows.push({
            key: String(cursor.primaryKey),
            core: mail.core,
            moved: !!mail.moved,
            local: !!mail.local,
            pendingMove: !!mail.pendingMove,
            syncReady: !!mail.reply && mail.sentMessageId !== undefined,
            sentMessageId: mail.sentMessageId,
          });
          if (rows.length === 50)
            resolve({ rows, next: String(cursor.primaryKey) });
          else cursor.continue();
        };
      });
    });
  }
}

import type { Account, Outgoing } from "./provider";
import type { LocalStore } from "./storage";

export interface SentReceipt {
  folder: string;
  remote_id: string | null;
}
export interface SentWork {
  state:
    | "pending"
    | "missing"
    | "reserved"
    | "copying"
    | "saved"
    | "failed"
    | "uncertain"
    | "unknown"
    | "local";
  folder?: string;
  copyId?: string;
  copyAccount?: Account;
  receipt?: SentReceipt;
}
export interface SentContext {
  store: LocalStore;
  pending: Map<string, SentWork>;
  response(path: string, body?: unknown): Promise<Response>;
  json(path: string, body?: unknown): Promise<Record<string, unknown>>;
  connection(account: Account): { account: Account; password: string };
  finishLocal(record: Outgoing): Promise<void>;
}
function requireValue(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}
function folder(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 1024 &&
    !/[\r\n\0]/.test(value)
  );
}
function receipt(value: unknown, destination: string): SentReceipt {
  requireValue(
    value && typeof value === "object",
    "Sent returned an invalid acknowledgment. Keep the local copy and check Outbox.",
  );
  const r = value as SentReceipt;
  requireValue(
    r.folder === destination &&
      (r.remote_id === null ||
        (typeof r.remote_id === "string" &&
          /^[1-9]\d*\.[1-9]\d*$/.test(r.remote_id))),
    "The Sent acknowledgment has a different destination or identity. Check Outbox before another upload.",
  );
  return { folder: r.folder, remote_id: r.remote_id };
}
export function sameIncoming(a: Account, b: Account) {
  return (
    [
      "id",
      "protocol",
      "host",
      "port",
      "username",
      "email",
      "incoming_security",
      "incoming_auth",
    ] as const
  ).every((k) => a[k] === b[k]);
}
/** Called under the profile's draft and account Web Locks. Only transport state
 * is transient; each upload identity/destination is committed before POST. */
export async function recoverSent(
  c: SentContext,
  record: Outgoing,
  action: "check" | "copy",
  confirmed: boolean,
  automatic: boolean,
) {
  async function save() {
    await c.store.commit([
      { store: "outgoing", key: record.draft.id, value: record },
    ]);
  }
  async function acknowledge(r: SentReceipt) {
    record.sent = {
      ...record.sent,
      state: "saved",
      folder: r.folder,
      receipt: r,
    };
    c.pending.set(record.id, structuredClone(record.sent));
    try {
      await save();
    } catch {
      throw new Error(
        "The server saved the Sent copy, but its acknowledgment could not be stored. Check Outbox to finish saving it; do not upload again.",
      );
    }
    c.pending.delete(record.id);
    try {
      await c.finishLocal(record);
    } catch {
      throw new Error(
        "The server saved the Sent copy, but the local cache needs repair. Check Outbox to finish; do not upload again.",
      );
    }
  }
  const pending = c.pending.get(record.id);
  if (pending?.receipt) return acknowledge(pending.receipt);
  if (record.sent?.state === "saved" || record.sent?.state === "local") {
    await c.finishLocal(record);
    return;
  }
  requireValue(
    !record.recovery || record.recovery.action === "marked",
    "This submission was already reviewed. Open its recovered draft or local copy.",
  );
  requireValue(
    ["delivered", "uncertain", "unknown"].includes(record.state),
    "Check delivery status before working on its Sent copy.",
  );
  requireValue(
    record.account && record.wire && record.mail,
    "This older submission has no saved account connection or exact MIME. Keep its local copy and review Sent with your provider.",
  );
  const saved = record.account;
  const known =
    record.state === "delivered" || record.recovery?.action === "marked";
  requireValue(
    action !== "copy" || known,
    "Review delivery before saving a server Sent copy. Uploading a copy does not send mail to recipients.",
  );
  if (
    automatic &&
    (saved.protocol === "Pop3" || saved.sent_copy === "LocalOnly")
  ) {
    record.sent = { state: "local" };
    await save();
    await c.finishLocal(record);
    return;
  }
  requireValue(saved.protocol === "Imap", "POP3 keeps Sent copies locally.");

  // Status does not require mailbox credentials, including after a reconnect or
  // settings change. A known acknowledgment wins over a later unavailable UID.
  let previous = record.sent;
  if (previous?.copyId) {
    const response = await c.response(`/api/mail/sent/${previous.copyId}`);
    if (response.status === 404) {
      record.sent = { ...previous, state: "unknown" };
      await save();
    } else {
      requireValue(
        response.ok,
        "Could not check the Sent upload status. Retry Outbox before another upload.",
      );
      const value = await response.json();
      requireValue(
        value.id === previous.copyId &&
          ["reserved", "copying", "saved", "failed", "uncertain"].includes(
            value.state,
          ),
        "Invalid Sent upload status. Keep its reservation and retry Outbox.",
      );
      if (value.state === "saved")
        return acknowledge(receipt(value.receipt, previous.folder!));
      record.sent = { ...previous, state: value.state };
      await save();
      if (value.state === "copying") return;
    }
    previous = record.sent;
  }
  const current = await c.store.get<Account>("accounts", saved.id);
  requireValue(
    current && sameIncoming(current, saved),
    "This account changed after sending. Restore its original incoming connection or keep the local Sent copy.",
  );
  const account = { ...current };
  if (previous?.copyId) {
    requireValue(
      folder(previous.folder),
      "The previous Sent upload has no destination. Keep its local copy and review the provider folder.",
    );
    account.sent_folder = previous.folder;
  }
  const connection = c.connection(account);
  const found = await c.json("/api/mail/sent/check", {
    connection,
    message_id: `<${record.id}@shep.so>`,
  });
  requireValue(
    folder(found.folder),
    "The server returned an invalid Sent destination.",
  );
  if (previous?.copyId)
    requireValue(
      found.folder === previous.folder,
      "The previous upload's Sent destination changed. Keep its local copy.",
    );
  if (found.receipt !== null)
    return acknowledge(receipt(found.receipt, found.folder));
  record.sent = {
    ...previous,
    state: previous?.copyId ? previous.state : "missing",
    folder: found.folder,
  };
  await save();
  if (
    action === "check" ||
    (automatic && account.sent_copy === "ServerManaged")
  )
    return;
  requireValue(
    account.sent_copy === "Automatic",
    "This account does not upload Sent copies. Change its Sent-copy preference, or keep the local copy.",
  );
  const retry = !!previous?.copyId && previous.state !== "reserved";
  requireValue(
    !retry || confirmed,
    "The previous Sent upload is not confirmed. Review the server folder and confirm before uploading another copy.",
  );
  let copyAccount =
    previous?.state === "reserved" ? previous.copyAccount : undefined;
  let copyId = previous?.state === "reserved" ? previous.copyId : undefined;
  if (!copyId) {
    copyAccount = { ...account, sent_folder: found.folder };
    const reserved = await c.json("/api/mail/sent/reserve", {
      connection: c.connection(copyAccount),
      wire: record.wire,
      timestamp: record.mail.core.timestamp,
      reviewed_retry: confirmed && retry,
    });
    requireValue(
      typeof reserved.id === "string" &&
        /^[A-Za-z0-9_-]{43}$/.test(reserved.id) &&
        ["reserved", "copying", "saved", "failed", "uncertain"].includes(
          String(reserved.state),
        ),
      "Could not reserve the Sent copy. No new upload was started.",
    );
    copyId = reserved.id;
    record.sent = {
      state: reserved.state as SentWork["state"],
      copyId,
      copyAccount,
      folder: found.folder,
    };
    if (reserved.state === "saved")
      return acknowledge(receipt(reserved.receipt, found.folder));
    await save();
    if (reserved.state !== "reserved") return;
  }
  requireValue(
    copyAccount &&
      sameIncoming(copyAccount, current) &&
      copyAccount.sent_folder === found.folder,
    "The saved Sent reservation has different connection settings. Keep its local copy and review Outbox.",
  );
  // The durable copying marker survives a tab close, unavailable browser storage
  // after acknowledgment, or an HTTP response that never reaches this tab.
  record.sent = { ...record.sent, state: "copying" };
  await save();
  let value: Record<string, unknown>;
  try {
    value = await c.json(`/api/mail/sent/${copyId}/copy`, {
      connection: c.connection(copyAccount),
      wire: record.wire,
      timestamp: record.mail.core.timestamp,
    });
    requireValue(
      value.id === copyId &&
        ["copying", "saved", "failed", "uncertain"].includes(
          String(value.state),
        ),
      "Invalid Sent result.",
    );
  } catch {
    throw new Error(
      "The Sent upload is not confirmed. Its saved reservation is kept; check Outbox before reviewing another upload.",
    );
  }
  if (value.state === "saved")
    return acknowledge(receipt(value.receipt, found.folder));
  record.sent = { ...record.sent, state: value.state as SentWork["state"] };
  await save();
  if (value.state !== "copying")
    throw new Error(
      typeof value.error === "string"
        ? value.error
        : "Sent was not confirmed. Check Outbox before reviewing another upload.",
    );
}

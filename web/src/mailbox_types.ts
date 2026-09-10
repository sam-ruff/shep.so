import type { Fields, Mail } from "./model";
import type { SelectionScope } from "./selection_types";
import type { CoreMail } from "./provider";

export interface MailboxQuery {
  scope: SelectionScope;
  /** Read-only counterfactual for one reviewed group; never authorizes execution. */
  undo?: string;
  previewOnly?: boolean;
  offset: number;
  observed?: string[];
  /** UI generation; not part of the persistent search predicate. */
  generation?: number;
}
export interface MailboxPage {
  undo?: {
    id: string;
    revision: number;
    committed: boolean;
    textMatches: Record<string, boolean>;
    beforeFields: Record<string, Fields>;
  };
  bulkRevision?: string;
  groupFields?: Record<string, Fields>;
  revision: number;
  epoch: string;
  rows: Mail[];
  total: number;
  unread: number;
  aliases: Record<string, string>;
  confirmed: Record<string, Fields>;
}
export interface MailboxMetadata {
  id: string;
  mail?: Mail;
  epoch: string;
  revision: number;
}
export interface MailboxDetail {
  id: string;
  body: string;
  revision: number;
  epoch: string;
}
export interface MailboxRepository {
  page(query: MailboxQuery): Promise<MailboxPage>;
  detail(id: string): Promise<MailboxDetail>;
  metadata(id: string): Promise<MailboxMetadata>;
  prefetch?(id: string): Promise<MailboxDetail>;
  close(): Promise<void>;
}
/** Only small cache/protocol metadata may cross the scan worker boundary. */
export interface MailScanEntry {
  key: string;
  core: CoreMail;
  moved: boolean;
  local: boolean;
  pendingMove: boolean;
  syncReady: boolean;
  sentMessageId?: string | null;
}
export interface MailScanQuery {
  account: string;
  folder?: string;
  serverId?: string;
  after?: string | null;
}
export interface MailScanPage {
  rows: MailScanEntry[];
  next: string | null;
}
export function checkedQuery(value: MailboxQuery): MailboxQuery {
  if (
    !value?.scope ||
    typeof value.scope.folder !== "string" ||
    !Number.isSafeInteger(value.offset) ||
    value.offset < 0
  )
    throw Error("Invalid mailbox page. Open the folder again.");
  if (value.undo !== undefined && !/^[\w-]{1,128}$/.test(value.undo))
    throw Error("Invalid group preview. Reopen History.");
  const s = value.scope;
  if (
    value.observed &&
    (!Array.isArray(value.observed) ||
      value.observed.length > 50 ||
      value.observed.some((id) => typeof id !== "string"))
  )
    throw Error("Observe one mailbox page at a time.");
  if (
    (s.account != null && typeof s.account !== "string") ||
    (s.oldest != null && typeof s.oldest !== "boolean") ||
    (s.query != null && typeof s.query !== "string") ||
    (s.projection != null &&
      (typeof s.projection !== "object" || Array.isArray(s.projection))) ||
    (s.filter != null && !["", "All", "Unread", "Flagged"].includes(s.filter))
  )
    throw Error("Invalid mailbox query. Open the folder again.");
  const projection: Record<string, Fields> = Object.create(null);
  for (const [id, fields] of Object.entries(s.projection ?? {})) {
    if (!id || !fields || typeof fields !== "object" || Array.isArray(fields))
      throw Error("Invalid pending mail change. Refresh the folder.");
    for (const [key, field] of Object.entries(fields)) {
      if (
        key === "folder"
          ? typeof field !== "string"
          : !["unread", "starred"].includes(key) || typeof field !== "boolean"
      )
        throw Error("Invalid pending mail change. Refresh the folder.");
    }
    projection[id] = { ...fields };
  }
  return {
    offset: value.offset,
    undo: value.undo,
    observed: value.observed?.slice(),
    scope: { ...s, projection },
  };
}

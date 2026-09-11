// Browser mirror of shared/profile-core's portable operation format and the
// history command contract. Passwords, OAuth grants and device journals never
// appear in these records.
export const PROFILE_FORMAT = "so.shep.profile-operation";
export const MAX_RECORD_BYTES = 1024 * 1024;
export const MAX_CHANGES = 64;
export const PAGE_SIZE = 50;
export const APPLY_BATCH = 32;

export interface PortableConnection {
  id: string;
  email: string;
  protocol: "Imap" | "Pop3";
  host: string;
  port: number;
  username: string;
  incoming_security: "Tls" | "StartTls";
  incoming_auth: "Password" | "Plain";
  smtp_host: string;
  smtp_port: number;
  smtp_username: string;
  smtp_security: "Tls" | "StartTls";
  smtp_auth: "Automatic" | "Plain" | "Login" | "None";
  smtp_separate_password: boolean;
  sent_copy: "Automatic" | "ServerManaged" | "LocalOnly";
  sent_folder: string;
  [extension: string]: unknown;
}
export type SettingKey =
  | "appearance"
  | "reply_display"
  | "image_policy"
  | "unified_inbox"
  | "cross_account_moves"
  | "group_conversations"
  | "desktop_badges"
  | "preview_lines"
  | "left_swipe"
  | "right_swipe"
  | "sender_pictures"
  | "tooltips";
export type Action =
  | { kind: "account_connection"; account: PortableConnection }
  | { kind: "account_name"; id: string; name: string }
  | { kind: "account_removed"; id: string }
  | { kind: "setting"; key: SettingKey; value: unknown }
  | { kind: "setting_removed"; key: SettingKey }
  | { kind: "profile_name"; name: string }
  | { kind: "profile_removed" }
  | { kind: "profile_setup"; complete: boolean };
export type Change = Action & { [extension: string]: unknown };
export interface Operation {
  format: string;
  major: number;
  minor: number;
  requires: string[];
  namespace: string;
  profile: string;
  generation: string;
  device: string;
  operation: string;
  parents: string[];
  changes: Change[];
  [extension: string]: unknown;
}

export interface Binding {
  namespace: string;
  principal: string;
  profile: string;
  generation: string;
}
export interface Resolution {
  target: string;
  versions: string[];
}
export interface LocalEdit {
  operation: string;
  expected_revision: number;
  changes: Change[];
  resolutions?: Resolution[];
}
export type HistoryCommand =
  | { kind: "state" }
  | { kind: "import"; record: string }
  | { kind: "edit"; edit: LocalEdit }
  | { kind: "drain" }
  | { kind: "fields"; after: string | null }
  | { kind: "versions"; target: string; after: string | null }
  | { kind: "value"; target: string; operation: string }
  | { kind: "export_record"; expected_revision: number; after: number }
  | {
      kind: "export_acknowledged_record";
      expected_revision: number;
      after: number;
    }
  | { kind: "next_upload" }
  | { kind: "reserve"; operation: string; file_id: string }
  | { kind: "confirm"; operation: string; file_id: string; sha256: string };
export interface HistoryState {
  device: string;
  revision: number;
  operations: number;
  waiting: number;
  ready: number;
  queued: number;
  fields: number;
  conflicts: number;
  removed: boolean;
  initialized: boolean;
}
export interface Overview {
  state: HistoryState;
  name: string | null;
  name_conflict: boolean;
  accounts: number;
  settings: number;
}
export interface Field {
  target: string;
  versions: number;
  conflict: boolean;
  revision: number;
}
export interface Version {
  operation: string;
  device: string;
}
export interface Upload {
  operation: string;
  record: string;
  sha256: string;
  file_id: string | null;
}
export interface ExportedRecord {
  position: number;
  operation: string;
  record: string;
}
export type HistoryReply =
  | { kind: "state"; value: HistoryState }
  | { kind: "fields"; value: Field[] }
  | { kind: "versions"; value: Version[] }
  | { kind: "value"; value: Change }
  | { kind: "upload"; value: Upload | null }
  | { kind: "record"; value: ExportedRecord | null };
/// One durable journal record exactly as the WASM journal stores it.
export interface StoredRecord {
  seq: number;
  operation: string;
  raw: string;
  request?: string;
  file_id?: string;
  uploaded: boolean;
}
export type HistoryErrorKind =
  | "binding"
  | "storage"
  | "owned"
  | "changed"
  | "conflict"
  | "removed"
  | "incomplete"
  | "identity"
  | "cycle"
  | "heads"
  | "busy"
  | "stopped"
  | "invalid"
  | "too_large"
  | "upgrade"
  | "local_data";
export class HistoryError extends Error {
  constructor(
    public readonly kind: HistoryErrorKind,
    message: string,
  ) {
    super(message);
  }
}

export function bindingKey(binding: Binding): string {
  return [
    binding.namespace,
    binding.principal,
    binding.profile,
    binding.generation,
  ].join("|");
}
export function targetOf(action: Action): string {
  switch (action.kind) {
    case "account_connection":
      return `account:${action.account.id}:connection`;
    case "account_name":
      return `account:${action.id}:name`;
    case "account_removed":
      return `account:${action.id}:removed`;
    case "setting":
    case "setting_removed":
      return `setting:${action.key}`;
    case "profile_name":
      return "profile:name";
    case "profile_removed":
      return "profile:removed";
    case "profile_setup":
      return "profile:setup";
  }
}
export const UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
export function isUuid(value: unknown): value is string {
  return (
    typeof value === "string" &&
    UUID_PATTERN.test(value) &&
    value !== "00000000-0000-0000-0000-000000000000"
  );
}

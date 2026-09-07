import type { Fields } from "./model";

export interface SelectionScope {
  folder: string;
  account?: string | null;
  query?: string;
  filter?: string;
  oldest?: boolean;
  projection?: Record<string, Fields>;
}
export type SelectionChange =
  | { kind: "set"; id: string; selected: boolean; clear_others: boolean }
  | { kind: "range"; anchor: string; target: string; additive: boolean }
  | { kind: "clear" };
export type SelectionCommand =
  | {
      kind: "capture";
      id: string;
      revision: number;
      scope: SelectionScope;
      all: boolean;
    }
  | {
      kind: "change";
      id: string;
      expected: number;
      scope: SelectionScope;
      change: SelectionChange;
    }
  | { kind: "observe"; id: string }
  | { kind: "freeze"; id: string; expected: number; target: string }
  | { kind: "page"; id: string; expected: number; after?: number | null }
  | { kind: "release"; id: string };
export interface SelectionGroup {
  account: string;
  folder: string;
  total: number;
  unread: number;
  starred: number;
}
export interface SelectionSnapshot {
  id: string;
  revision: number;
  frozen: boolean;
  total: number;
  selected: number;
  available: number;
  unread: number;
  starred: number;
  groups: SelectionGroup[];
  visible: string[];
  positions: Record<string, number>;
  aliases: Record<string, string>;
}
export interface SelectionPage {
  revision: number;
  rows: {
    position: number;
    id: string;
    account: string;
    folder: string;
    unread: boolean;
    starred: boolean;
  }[];
  next_after: number | null;
}
export type SelectionResult = SelectionSnapshot | SelectionPage | null;
export interface SelectionRepository {
  selection(
    command: SelectionCommand,
    observed?: string[],
  ): Promise<SelectionResult>;
}
export const selectionScope = (scope: SelectionScope): SelectionScope => ({
  folder: scope.folder.toLowerCase() === "inbox" ? "INBOX" : scope.folder,
  account: scope.account ?? null,
  query: scope.query ?? "",
  filter: scope.filter ?? "",
  oldest: scope.oldest ?? false,
});
export function selectionToken(value: string) {
  if (typeof value !== "string" || !/^[\w-]{1,128}$/.test(value))
    throw Error("Invalid selection identity. Select the messages again.");
}

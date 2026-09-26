// Move chooser destinations: the acted account's folders plus, while the user
// types, matching folders from other IMAP accounts. Ranking runs in the shared
// Rust matcher so the browser and desktop order rows identically.

/** One connected account as the chooser sees it, in the account listing order. */
export interface MoveAccount {
  id: string;
  name: string;
  email: string;
  imap: boolean;
  /** Displayed folder names; undefined until the account's folders are known. */
  folders?: string[];
}
export interface MoveCandidate {
  account: string;
  folder: string;
  label: string;
  foreign: boolean;
  /** Position of the account in the listing, the final tie-break. */
  order: number;
}
/** Where the unbadged folders come from. */
export type MoveSource =
  | { kind: "message"; account: string | null }
  | { kind: "selection"; accounts: string[] | null };
export interface MoveHome {
  /** An explicitly chosen destination account, if any. */
  explicit?: string | null;
  source: MoveSource;
}
/** Shared Rust ranking: JSON candidates in, JSON positions out. */
export type Ranker = (query: string, candidates: string) => string;

/** IMAP accounts other than the home ones. Nothing is foreign without a home
 * account, and a POP3 source never reaches another account. */
export function foreignAccounts(
  accounts: MoveAccount[],
  home: string[],
  enabled: boolean,
) {
  if (!enabled || !home.length) return [];
  const imap = (id: string) => accounts.some((a) => a.id === id && a.imap);
  if (!home.every(imap)) return [];
  return accounts.filter((a) => a.imap && !home.includes(a.id));
}

function homeFolders(
  accounts: MoveAccount[],
  home: MoveHome,
  fallback: string[],
): [string[], string[]] {
  const own = (id: string | null) =>
    accounts.find((a) => a.id === id)?.folders ?? fallback;
  if (home.explicit) return [[home.explicit], own(home.explicit)];
  const source = home.source;
  if (source.kind === "message")
    return [source.account ? [source.account] : [], own(source.account)];
  if (!source.accounts?.length) return [[], []];
  const [first, ...rest] = source.accounts;
  let folders = [...own(first)];
  for (const id of rest) {
    const other = own(id);
    folders = folders.filter((folder) => other.includes(folder));
  }
  return [source.accounts, folders];
}

/** The standard folders plus every known folder, offered for an account
 * without a server folder list (POP3, or not loaded yet). */
export function knownFolders(accounts: MoveAccount[]) {
  return [
    ...new Set([
      "Inbox",
      "Archive",
      "Sent",
      "Trash",
      "Spam",
      ...accounts.flatMap((a) => a.folders ?? []),
    ]),
  ];
}

const position = (accounts: MoveAccount[], id: string) => {
  const index = accounts.findIndex((a) => a.id === id);
  return index < 0 ? Number.MAX_SAFE_INTEGER : index;
};

/** Home rows first, in catalogue order, then one row per foreign account and
 * folder. Foreign rows appear only while typing and never for an explicit
 * destination account. */
export function gather(
  accounts: MoveAccount[],
  home: MoveHome,
  fallback: string[],
  enabled: boolean,
  queryIsEmpty: boolean,
): MoveCandidate[] {
  const [homeAccounts, folders] = homeFolders(accounts, home, fallback);
  const homeId = homeAccounts[0] ?? "";
  const candidates: MoveCandidate[] = [...new Set(folders)].map((folder) => ({
    account: homeId,
    folder,
    label: folder,
    foreign: false,
    order: position(accounts, homeId),
  }));
  if (queryIsEmpty || home.explicit) return candidates;
  for (const account of foreignAccounts(accounts, homeAccounts, enabled))
    for (const folder of new Set(account.folders ?? []))
      candidates.push({
        account: account.id,
        folder,
        label: folder,
        foreign: true,
        order: position(accounts, account.id),
      });
  return candidates;
}

/** Rank with the shared matcher, adding its foreign penalty to foreign rows. */
export function rank(
  query: string,
  candidates: MoveCandidate[],
  ranker: Ranker,
): MoveCandidate[] {
  const order: unknown = JSON.parse(
    ranker(
      query,
      JSON.stringify(
        candidates.map(({ label, folder, foreign, order }) => ({
          label,
          folder,
          foreign,
          order: Math.min(order, 2 ** 31),
        })),
      ),
    ),
  );
  if (
    !Array.isArray(order) ||
    order.some((i) => !Number.isInteger(i) || !candidates[i])
  )
    throw Error("Could not rank folders. Reload Shep and retry.");
  return order.map((i: number) => candidates[i]);
}

/** The sidebar's account name: the email unless another account shares it,
 * then the account name. */
export function accountDisplay(
  accounts: Pick<MoveAccount, "id" | "name" | "email">[],
  id: string,
) {
  const account = accounts.find((a) => a.id === id);
  if (!account) return undefined;
  const duplicated =
    accounts.filter((a) => a.email === account.email).length > 1;
  return duplicated && account.name.trim() ? account.name : account.email;
}

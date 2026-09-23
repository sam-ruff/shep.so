import { it, expect, beforeAll, describe } from "vitest";
import { readFileSync } from "node:fs";
import { initSync, rank_move_candidates } from "./wasm/shep_mail_content";
import cases from "../../shared/move-ranking-cases.json";
import {
  accountDisplay,
  foreignAccounts,
  gather,
  rank,
  type MoveAccount,
  type MoveHome,
} from "./move_candidates";

beforeAll(() =>
  initSync({
    module: new WebAssembly.Module(
      readFileSync(
        new URL("./wasm/shep_mail_content_bg.wasm", import.meta.url),
      ),
    ),
  }),
);

const accounts = (): MoveAccount[] => [
  {
    id: "work",
    name: "work name",
    email: "alex@studio.example",
    imap: true,
    folders: ["Inbox", "Archive", "Archives", "Projects/Archive"],
  },
  {
    id: "personal",
    name: "personal name",
    email: "alex@example.com",
    imap: true,
    folders: ["Inbox", "Archive", "Home.Plans"],
  },
  {
    id: "club",
    name: "club name",
    email: "alex@club.example",
    imap: true,
    folders: ["Inbox", "Home.Plans"],
  },
  {
    id: "pop",
    name: "pop name",
    email: "alex@pop.example",
    imap: false,
    folders: ["Inbox", "Archive"],
  },
];
const message = (account: string): MoveHome => ({
  source: { kind: "message", account },
});
const ranked = (query: string, home: MoveHome, enabled = true) =>
  rank(
    query,
    gather(accounts(), home, [], enabled, !query.trim()),
    rank_move_candidates,
  ).map((c) => [c.account, c.folder, c.foreign]);

describe("shared ranking", () => {
  it("orders every shared case exactly as the desktop does", () => {
    for (const c of cases)
      expect(
        JSON.parse(rank_move_candidates(c.query, JSON.stringify(c.candidates))),
        c.name,
      ).toEqual(c.expected);
  });
  it("rejects a ranker reply that names an unknown row", () => {
    const rows = gather(accounts(), message("work"), [], true, false);
    expect(() => rank("a", rows, () => "[99]")).toThrow(/Could not rank/);
  });
});

describe("gathering", () => {
  it("ranks identical names in the home account first", () => {
    expect(ranked("archive", message("work"))).toEqual([
      ["work", "Archive", false],
      ["work", "Projects/Archive", false],
      ["personal", "Archive", true],
      ["work", "Archives", false],
    ]);
  });
  it("lets a foreign exact match outrank home abbreviations and typos", () => {
    expect(ranked("arch", message("work")).map((r) => r[1])).toEqual([
      "Archive",
      "Archives",
      "Archive",
      "Projects/Archive",
    ]);
    expect(ranked("plans", message("work"))).toEqual([
      ["personal", "Home.Plans", true],
      ["club", "Home.Plans", true],
    ]);
    const typo = ranked("archvie", message("personal"));
    expect(typo[0]).toEqual(["personal", "Archive", false]);
    expect(typo[1]).toEqual(["work", "Archive", true]);
  });
  it("lists only home folders while the query is empty", () => {
    for (const query of ["", "   "]) {
      const rows = ranked(query, message("work"));
      expect(rows).toHaveLength(4);
      expect(rows.every(([account, , foreign]) => account === "work" && !foreign)).toBe(true);
    }
  });
  it("skips POP3 accounts and never badges the home account", () => {
    expect(ranked("archive", message("work")).some(([a]) => a === "pop")).toBe(false);
    expect(ranked("archive", message("pop")).some(([, , f]) => f)).toBe(false);
  });
  it("adds no foreign rows for an explicit destination account", () => {
    const home: MoveHome = {
      explicit: "personal",
      source: { kind: "message", account: "work" },
    };
    expect(ranked("archive", home)).toEqual([["personal", "Archive", false]]);
  });
  it("intersects a selection's folders and adds foreign rows only when all are IMAP", () => {
    const home: MoveHome = {
      source: { kind: "selection", accounts: ["work", "personal"] },
    };
    expect(ranked("", home).map((r) => r[1])).toEqual(["Archive", "Inbox"]);
    expect(ranked("plans", home)).toEqual([["club", "Home.Plans", true]]);
    const mixed: MoveHome = {
      source: { kind: "selection", accounts: ["work", "pop"] },
    };
    expect(ranked("plans", mixed)).toEqual([]);
    const none: MoveHome = { source: { kind: "selection", accounts: null } };
    expect(ranked("plans", none)).toEqual([]);
  });
  it("adds nothing foreign when the preference is off", () => {
    expect(ranked("plans", message("work"), false)).toEqual([]);
    expect(foreignAccounts(accounts(), [], true)).toEqual([]);
  });
  it("falls back to the known folder list for an uncatalogued account", () => {
    const rows = gather(
      [{ id: "new", name: "", email: "n@example.test", imap: true }],
      message("new"),
      ["Inbox", "Archive"],
      true,
      true,
    );
    expect(rows.map((r) => r.folder)).toEqual(["Inbox", "Archive"]);
  });
});

it("names accounts by email unless another account shares it", () => {
  const list = accounts();
  expect(accountDisplay(list, "work")).toBe("alex@studio.example");
  list[2].email = "alex@studio.example";
  expect(accountDisplay(list, "work")).toBe("work name");
  list[0].name = " ";
  expect(accountDisplay(list, "work")).toBe("alex@studio.example");
  expect(accountDisplay(list, "missing")).toBeUndefined();
});

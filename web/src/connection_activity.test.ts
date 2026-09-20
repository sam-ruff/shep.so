import "fake-indexeddb/auto";
import { expect, test } from "vitest";
import { BrowserStore } from "./storage";
import { connectionActivity, connectionStatus } from "./connection_activity";
import type { AccountConnection } from "./provider";

test("connection activity reads a bounded persisted page without secrets or a second owner", async () => {
  const store = await BrowserStore.open("activityConnections".padEnd(43, "x"));
  try {
    await store.commit(Array.from({ length: 25 }, (_, i) => ({
      store: "accountConnections" as const, key: String(i).padStart(2, "0"),
      value: { id: `attempt-${i}`, account: { id: `account-${i}`, email: `${i}@example.test` }, state: "failed", error: "Probe refused" },
    })));
    const limits: (number | undefined)[] = [];
    const page = await connectionActivity({ connectionProgress: limit => {
      limits.push(limit); return store.all<AccountConnection>("accountConnections", limit);
    } });
    expect(limits).toEqual([21]);
    expect(page.rows).toHaveLength(20);
    expect(page.more).toBe(true);
    expect(page.rows[0].id).toBe("attempt-0");
    expect(await store.all("accountConnections", 21)).toHaveLength(21);
    expect(await store.all("accountConnections")).toHaveLength(25);
  } finally { store.close(); }
});

test("unconfirmed connection progress does not claim activation or automatic credential retry", () => {
  const attempt = { id: "attempt", account: { email: "owner@example.test" }, state: "checking" } as AccountConnection;
  expect(connectionStatus(attempt)).toContain("not yet confirmed");
  expect(connectionStatus(attempt)).toContain("re-enter your password");
  expect(connectionStatus({ ...attempt, state: "failed" })).toContain("has not been replaced");
});

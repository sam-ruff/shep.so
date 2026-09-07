import { it, expect, beforeAll, vi } from "vitest";
import { readFileSync } from "node:fs";
import { initSync, find_text } from "./wasm/shep_mail_content";
import cases from "../../shared/find-cases.json";
import { MessageFind, type SearchHit } from "./message_find";
import { BrowserSettings } from "./model";
beforeAll(() =>
  initSync({
    module: new WebAssembly.Module(
      readFileSync(
        new URL("./wasm/shep_mail_content_bg.wasm", import.meta.url),
      ),
    ),
  }),
);
it("WASM preserves the desktop whitespace, Unicode, literal and UTF-16 contract", () => {
  for (const c of cases)
    expect(
      JSON.parse(find_text(JSON.stringify(c.blocks), c.query, c.match_case)),
    ).toEqual(c.hits);
});
it("coalesces pending searches and rejects old message, scope, query, error and close results", async () => {
  vi.useFakeTimers();
  const jobs: {
    blocks: string[];
    query: string;
    resolve: (hits: SearchHit[]) => void;
    reject: (error: Error) => void;
  }[] = [];
  const find = new MessageFind(
    (blocks, query) =>
      new Promise((resolve, reject) =>
        jobs.push({ blocks, query, resolve, reject }),
      ),
  );
  try {
    find.setSource("first", ["old"]);
    find.show();
    find.setQuery("old");
    await vi.advanceTimersByTimeAsync(100);
    expect(jobs).toHaveLength(1);
    find.setSource("second", ["latest"]);
    find.setQuery("discarded");
    find.setQuery("latest");
    await vi.advanceTimersByTimeAsync(100);
    expect(jobs).toHaveLength(1);
    jobs[0].reject(new Error("obsolete"));
    await vi.advanceTimersByTimeAsync(0);
    expect(find.error).toBeNull();
    expect(jobs).toHaveLength(2);
    expect(jobs[1].blocks).toEqual(["latest"]);
    expect(jobs[1].query).toBe("latest");
    jobs[1].resolve([
      { block: 0, start: 0, end: 2 },
      { block: 0, start: 2, end: 4 },
    ]);
    await vi.advanceTimersByTimeAsync(0);
    expect(find.status).toBe("1 of 2");
    find.next(true);
    expect(find.status).toBe("2 of 2");
    find.next();
    expect(find.status).toBe("1 of 2");
    find.toggleCase();
    await vi.advanceTimersByTimeAsync(100);
    find.close();
    jobs[2].resolve([{ block: 0, start: 0, end: 6 }]);
    await vi.advanceTimersByTimeAsync(0);
    expect(find.hits).toEqual([]);
    expect(find.open).toBe(false);
    find.show();
    await vi.advanceTimersByTimeAsync(100);
    jobs[3].reject(new Error("current"));
    await vi.advanceTimersByTimeAsync(0);
    expect(find.error).toContain("Retry Find");
    find.retry();
    await vi.advanceTimersByTimeAsync(100);
    jobs[4].resolve([]);
    await vi.advanceTimersByTimeAsync(0);
    expect(find.error).toBeNull();
    expect(find.status).toBe("No matches");
  } finally {
    find.dispose();
    vi.useRealTimers();
  }
});

it("new Find defaults preserve existing shortcut assignments and deliberate disabling", () => {
  try {
    vi.stubGlobal("localStorage", {
      getItem: () =>
        JSON.stringify({ version: 1, shortcuts: { move: "Control+f" } }),
    });
    expect(new BrowserSettings().read().shortcuts).toMatchObject({
      move: "Control+f",
      find: "",
    });
    vi.stubGlobal("localStorage", {
      getItem: () => JSON.stringify({ version: 1, shortcuts: { find: "" } }),
    });
    expect(new BrowserSettings().read().shortcuts.find).toBe("");
  } finally {
    vi.unstubAllGlobals();
  }
});

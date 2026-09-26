import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { ProgressPacer } from "./progress_pacer";

function deferred() {
  let resolve!: () => void, reject!: (error: unknown) => void;
  const promise = new Promise<void>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}
beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

test("the first value is delivered at once and later values wait for the round", async () => {
  const rounds: ReturnType<typeof deferred>[] = [],
    delivered: number[] = [];
  const pacer = new ProgressPacer<number>(
    (value) => {
      delivered.push(value);
      const round = deferred();
      rounds.push(round);
      return round.promise;
    },
    100,
    () => Date.now(),
  );
  pacer.push(1);
  expect(delivered).toEqual([1]);
  pacer.push(2);
  pacer.push(3);
  await vi.advanceTimersByTimeAsync(1000);
  // The consumer is still reading, so nothing more is started.
  expect(delivered).toEqual([1]);
  rounds[0].resolve();
  // A round that took a second is followed by a gap of a second.
  await vi.advanceTimersByTimeAsync(999);
  expect(delivered).toEqual([1]);
  await vi.advanceTimersByTimeAsync(1);
  // Only the latest value is delivered; the superseded one is dropped.
  expect(delivered).toEqual([1, 3]);
});

test("a quick round keeps the minimum gap, and an idle pacer delivers at once", async () => {
  const delivered: number[] = [];
  const pacer = new ProgressPacer<number>(
    (value) => {
      delivered.push(value);
    },
    100,
    () => Date.now(),
  );
  pacer.push(1);
  pacer.push(2);
  await vi.advanceTimersByTimeAsync(99);
  expect(delivered).toEqual([1]);
  await vi.advanceTimersByTimeAsync(1);
  expect(delivered).toEqual([1, 2]);
  // No newer value arrived during the next gap, so the pacer goes idle.
  await vi.advanceTimersByTimeAsync(500);
  pacer.push(3);
  expect(delivered).toEqual([1, 2, 3]);
});

test("a failed or throwing round still ends, and close drops pending values", async () => {
  const round = deferred(),
    delivered: number[] = [];
  let throwing = false;
  const pacer = new ProgressPacer<number>(
    (value) => {
      delivered.push(value);
      if (throwing) throw Error("fixture render failure");
      return value === 1 ? round.promise : undefined;
    },
    100,
    () => Date.now(),
  );
  pacer.push(1);
  pacer.push(2);
  round.reject(Error("fixture read failure"));
  await vi.advanceTimersByTimeAsync(100);
  expect(delivered).toEqual([1, 2]);
  throwing = true;
  pacer.push(3);
  await vi.advanceTimersByTimeAsync(100);
  expect(delivered).toEqual([1, 2, 3]);
  pacer.push(4);
  await vi.advanceTimersByTimeAsync(100);
  expect(delivered).toEqual([1, 2, 3, 4]);
  pacer.push(5);
  pacer.close();
  await vi.advanceTimersByTimeAsync(1000);
  pacer.push(6);
  await vi.advanceTimersByTimeAsync(1000);
  expect(delivered).toEqual([1, 2, 3, 4]);
});

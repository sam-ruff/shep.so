import { describe, expect, it } from "vitest";
import { clockTime, readerDate, rowDate } from "./format";

describe("desktop date formats", () => {
  const now = new Date(2026, 8, 9, 15, 4);
  it("shows the time for today's mail and day-month otherwise", () => {
    expect(rowDate(new Date(2026, 8, 9, 9, 5), now)).toBe("09:05");
    expect(rowDate(new Date(2026, 8, 6, 10, 42), now)).toBe("06 Sep");
  });
  it("formats the reader header like the desktop", () => {
    const date = new Date(2026, 8, 9, 15, 4);
    expect(readerDate(date)).toBe("09 Sep 2026");
    expect(clockTime(date)).toBe("15:04");
  });
});

import { afterEach, describe, expect, it, vi } from "vitest";
import { MoveFeedback } from "./move-feedback";

describe("desktop move notification contract", () => {
  afterEach(() => vi.useRealTimers());
  it("groups standard destinations across accounts, restarts expiry and never revives dismissal", () => {
    vi.useFakeTimers();
    const feedback = new MoveFeedback(() => {});
    const first = feedback.add("1", "work", "Inbox", "Archive");
    expect(feedback.label).toBe("Archived 1 message");
    vi.advanceTimersByTime(3000);
    feedback.add("2", "personal", "Inbox", "archive");
    vi.advanceTimersByTime(3000);
    expect(feedback.label).toBe("Archived 2 messages");
    feedback.failed(first);
    expect(feedback.label).toBe("Archived 1 message");
    vi.advanceTimersByTime(3000);
    expect(feedback.label).toBeNull();
    feedback.add("3", "work", "Inbox", "Plans");
    feedback.add("4", "personal", "Inbox", "Plans");
    expect(feedback.label).toBe("Moved 1 message to Plans");
    feedback.add("5", "personal", "Inbox", "Plans");
    expect(feedback.label).toBe("Moved 2 messages to Plans");
    feedback.add("6", "personal", "Plans", "INBOX");
    expect(feedback.label).toBe("Moved 1 message to Inbox");
    feedback.dismiss();
    feedback.failed(first);
    expect(feedback.label).toBeNull();
    feedback.dispose();
  });
});

import assert from "node:assert/strict";
import { withProfileHarness } from "./profile_harness.mjs";
await withProfileHarness(
  process.argv[2] ?? "web",
  "discovery",
  async ({ steps, labels, reveal, tap, wait, capture, theme }) => {
    await wait("A little room for good ideas");
    await tap("Preferences");
    await reveal("Saved Google profiles", { button: true });
    await tap("Saved Google profiles");
    await reveal("Find profiles", { button: true });
    await tap("Find profiles");
    await wait("Profile files could not be read");
    await capture("discovery-error-light");
    await reveal("Retry discovery", { button: true });
    await tap("Retry discovery");
    await wait("Profile discovery complete");
    await reveal("Personal");
    await capture("discovery-profiles-light");
    assert.ok((await labels()).includes("2 accounts"));
    steps.push("discovery-error-retry");
    await tap("Back");
    await theme("Dark");
    await reveal("Saved Google profiles", { button: true });
    await tap("Saved Google profiles");
    await reveal("Personal");
    await capture("discovery-profiles-dark");
    steps.push("discovery-dark-appearance");
    await reveal("Disconnect…", { button: true, direction: -1 });
    await tap("Disconnect…");
    await wait("Disconnect Google here?");
    await tap("Disconnect");
    await wait("Sign in and enable Drive");
    await reveal("Sign in and enable Drive");
    await capture("discovery-disconnected-dark");
    assert.ok(!(await labels()).includes("Profile discovery complete"));
    steps.push("discovery-disconnect");
    await tap("Back");
    await tap("Mail");
    await wait("A little room for good ideas");
    steps.push("discovery-mail-navigation");
  },
);

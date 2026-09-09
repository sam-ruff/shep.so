# Responsiveness contracts

Performance measurements must run on an otherwise idle machine. During development on a busy PC, use the functional tests and defer measurement until the end. Do not interpret contention-heavy results as a regression or change thresholds to accommodate them.

## Enforced gates

`performance-budgets.json` is the source of truth. `scripts/performance_gate.py` requires both backend and native UI reports, rejects missing/non-finite/negative results, requires at least 20 samples, and checks the 100,000-message backend dataset.

| Measurement | p95 ceiling |
| --- | ---: |
| Inbox page, 100,000 cached messages | 50 ms |
| Account-filtered page | 50 ms |
| Full-text search | 50 ms |
| Cached body load | 10 ms |
| UI update handler | 8 ms |
| Native tab navigation, input through observed state | 150 ms |

The backend benchmark uses a temporary SQLite WAL database with 100,000 messages over four accounts, 60 samples per query and 100 body loads. It also verifies that a full bounded channel returns backpressure immediately. It writes `artifacts/performance/backend.json`.

The native suite builds the optimized `test-ui` profile, launches a separate Xvfb display and in-memory fixture workspace, then performs 30 actual native shortcut navigations through the MCP harness. Elapsed time includes IPC, xdotool and state observation. Key injection explicitly uses a 1 ms inter-key delay, matching text injection; state polling is every 5 ms to keep measurement quantization below the gate margin; the xdotool default inserts 12 ms between chord events and measures artificial harness waiting. It writes `artifacts/performance/ui.json`. The combined gate requires both files. These checks are present in the disabled Linux CI job and in `scripts/check.sh`.

Handler timing is not a frame-rate measurement. Input-to-state timing includes dispatch and observation, but not proof that every pixel has reached the display. Screenshots after a short presentation wait provide visual evidence separately. Xvfb uses software rendering; GPU/display frame pacing and live-server throughput need separate profiling on the target machines. Aim for 60 Hz interaction and a 16.7 ms frame budget, without claiming it from handler timings alone.

## Implementation constraints

- UI callbacks never wait for database, filesystem, network, keychain, compression, cryptography or MIME operations.
- UI/backend channels have capacity 32, download channels 8, and explicit jobs at most 8. Account sync has at most 3 concurrent accounts. Mutations and sync for the same account serialize.
- Search debounces 100 ms and generation-checks responses. Pages contain 50 metadata rows; only visible rows plus a small margin are laid out, using a fixed row height.
- Adjacent bodies, hovered messages and the next page preload. Body cache is at most 8 entries / 32 MiB. Reader text is capped in the storage worker before it reaches the UI.
- Small WebP logos load once. SVG icons cache their handles. Drag resize events are coalesced every 16 ms; the final split is saved after a 350 ms pause.
- Network failures and full queues must produce visible feedback without disabling tab navigation, scrolling, search or appearance controls.

## Recording results

At the end of development, run the benchmark and native suite without concurrent builds, then run the combined gate. Save numerical evidence under `artifacts/performance/`, and record the machine, renderer, profile and date with any reported results. Test runs during known host saturation are diagnostic only and must be discarded before the final gate. Those contention-heavy runs are excluded from the baseline below.

## Validated baseline — 6 September 2026

Linux x86_64, iced tiny-skia on Xvfb at 1440×920. Backend: optimized release, 100,000 messages / four accounts, 60 query samples and 100 body loads. UI: optimized `test-ui`, 30 native tab transitions, no concurrent compilation. All 19 native scenarios (18 functional plus timing) passed.

| Measurement | Measured p95 | Gate |
| --- | ---: | ---: |
| Inbox page | 3.406 ms | 50 ms |
| Account page | 1.133 ms | 50 ms |
| Fuzzy full-text search | 35.952 ms | 50 ms |
| Cached body | 0.080 ms | 10 ms |
| UI handler | 0.008 ms | 8 ms |
| Native tab navigation | 145.004 ms | 150 ms |

The initial native runs measured 154.6–156.8 ms with coarser observation. The harness now explicitly uses 1 ms key injection and 5 ms state polling, which removes artificial key waiting and reduces timing quantization. Budgets were not changed. The native result has limited headroom and must be rechecked on the self-hosted runners. It measures input-to-observed-state, not display frame pacing; it does not establish 60 FPS or live network throughput.


## Client worktree checkpoint — 9 September 2026

The profile-history SQLite update prompted a fresh storage check. Relevance now
materializes exact-match ranks once; a covering unread-account index avoids
per-page message-row lookups and sorting. Release benchmark: 100,000 synthetic
messages/four accounts, 60 samples per query and 100 body loads, Linux x86_64.
Compilation and the owned Android emulator had stopped; ordinary desktop services
remained active. No fully idle-host claim is made.

| Measurement | Measured p95 | Gate |
| --- | ---: | ---: |
| Inbox page | 6.030 ms | 50 ms |
| Account page | 3.106 ms | 50 ms |
| Fuzzy full-text search | 35.537 ms | 50 ms |
| Cached body | 0.025 ms | 10 ms |
| UI handler, latest native run | 0.010 ms | 8 ms |
| Native tab navigation, latest run | **155.144 ms** | **150 ms — fails** |

**The combined performance gate fails.** Four native runs, each with 30 transitions,
measured navigation p95 at 154.81–162.33 ms. The cached test executable built before
this checkpoint also failed three runs at 159.31–168.24 ms. This comparison does
not identify the underlying cause or certify an exact earlier source commit. The
current test executable was restored and verified by hash; no installed app was
used. Retain R03/R09 and the native input-under-load audit.

The initial search benchmark was interrupted for diagnosis; the first complete
materialized-search run failed at 68.94 ms. Both remain recorded alongside the
passing final storage report. Native functional coverage passed 118 scenarios
before the final index and 14 relevant scenarios after it with compilation stopped.
The first 14-scenario rerun during compilation had two input failures; it is not a
pass. See [the completion log](COMPLETION.md) for evidence and shipping. No threshold
was weakened, and input-to-state timing is not proof of presented pixels or 60 Hz.

# Responsiveness contracts

Performance measurements must run on an otherwise idle machine. During development on a busy PC, use the functional tests and defer measurement until the end. Do not interpret contention-heavy results as a regression or change thresholds to accommodate them.

## Enforced gates

`performance-budgets.json` is the source of truth. `scripts/performance_gate.py` requires backend, native navigation and HTML pixel reports, rejects missing/non-finite/negative results, requires at least 20 samples, and checks the 100,000-message backend dataset.

| Measurement | p95 ceiling |
| --- | ---: |
| Inbox page, 100,000 cached messages | 50 ms |
| Account-filtered page | 50 ms |
| Full-text search | 50 ms |
| Cached body load | 10 ms |
| UI update handler | 8 ms |
| Native tab navigation, input through observed state | 150 ms |
| Unprepared HTML document, native click through displayed pixels | 100 ms |
| Cached/prefetched HTML, native click through displayed pixels | 50 ms |

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

## HTML opening — 7 September 2026

The user explicitly authorized this focused measurement before the remaining
idle-host gates. `scripts/html_latency.py` measures a native XTest click through
visible HTML pixels on a 1440×920 RGB24 Xvfb window, using the optimized `test-ui`
build. No builds ran concurrently with the reported comparison; the host was
not asserted fully idle. Each case has 20 independent observations.

| Opening case | Before p50 / p95 | After p50 / p95 |
| --- | ---: | ---: |
| Unprepared 200-paragraph letter | 87.5 / 122.4 ms | 39.9 / 45.8 ms |
| Return to styled mail | 84.2 / 103.3 ms | 25.1 / 25.7 ms |
| Prefetched adjacent message | 113.7 / 140.3 ms | 23.5 / 26.3 ms |
| Reopen the long letter | 95.7 / 104.8 ms | 26.5 / 37.0 ms |

Bounded width/glyph caching removes repeated shaping/rasterization. The reader
retains visited initial frames as well as adjacent preparations. Coalescing
overlapping damage and painting only visible solid panel interiors removes
repeated software painting of the same region. Pixel-equivalence tests protect
borders, text, shadows and fractional scaling; external-image policy remains
part of the frame-cache identity.

Reference pixels are prepared in a separate fixture process. Each measurement
process starts fresh; its initial styled message has already warmed the font
system. “Unprepared” describes the selected document, not process startup or
first font discovery. The styled sample has tables, inline WebP, CSS and blocked
external images. These figures do not establish live download speed, arbitrary
HTML complexity, other platforms, monitor scanout or sustained frame rate.

The sampler chooses 64 text/edge points across the visible body and waits for
at least 97% to match at RGB tolerance 8, with a 2 ms polling sleep. It rejects an
already visible reference. Pointer placement, reference preparation, dwell and
screenshots are outside the measured interval. The gate recomputes p95 from raw
observations and rejects invalid, repeated or insufficient samples. Run:

```sh
python3 scripts/html_latency.py --samples 20 --output artifacts/performance/html.json
python3 scripts/performance_gate.py --html-only
```

Full quality also runs these alongside backend/navigation gates. Reports and
WebP evidence stay under ignored `artifacts/`; no personal inbox is used.

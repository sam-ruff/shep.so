# Shared profile core distribution

This directory and the three adjacent JSON fixtures are copied from published
client commit `184b98a` without changing the initialization/history/Drive contract.
The nondefault `test-support` transport from `33d222d7` is retained for desktop's
owned loopback MCP fixture. It accepts only an explicit loopback HTTP port and a
fixed fake credential. Production builds do not expose that entry point.

This isolated distribution branch preserves the active client worktree. Desktop
consumes its immutable Git revision; it must not merge unrelated client sources.
The standalone all-feature suite passes 53 tests. Keep the format, initialization,
Drive and original fixed-token harness regressions when updating this snapshot.

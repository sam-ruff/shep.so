# Preferences search

The search demos share a static catalogue of section titles, actual control
captions, descriptions and common synonyms. Search includes controls in account,
calendar, Google, profile and backup forms. It indexes no personal names,
addresses, passwords, tokens or saved configuration values.

The common matcher normalises Latin accents, preserves other scripts and requires
every distinct query word. Exact section titles come first, followed by exact
control captions, caption prefixes and weighted word matches. Title words have
more weight than control captions, tab names, descriptions or synonyms. The
baseline uses RapidFuzz OSA with a bounded edit distance, including adjacent
transpositions. Numeric words match exactly. Stable title ordering resolves ties.
`matches_with` is the pure word-scoring boundary for the comparison branches.
The word matcher does not bridge abbreviations across separate caption words.

Static text is normalised once. Results are calculated when the input changes
and reused during drawing and observations. The ignored ranking measurement
checks complete warmed searches separately from native input and rendering;
run it only after other builds stop.

Selecting a result opens its existing section and never changes a preference.
Long cards can still require scrolling. Provider-specific backup controls need
the corresponding destination to be selected, and account/calendar fields need
their existing Edit or Add controls. Exact per-control reveal remains R99 work.

Tests exhaustively destructure `Preferences`, compare its serialised keys with
search coverage and documented exclusions, and audit nested network/configuration
schemas. New persisted fields must receive a coverage review. Excluded fields
are layout/sidebar state, mailbox sorting, per-message image decisions, provider
receipts, credential identifiers, legacy OAuth data and the old calendar interval.
Backup identity/receipt fields and calendar permission receipts are observed
metadata, not searchable values. Password controls are indexed by caption only.

Browser and Flutter Preferences search have not received this catalogue or its
ranking choices. Those parity gaps and exhaustive dynamic-control coverage remain
open; the demos do not choose a production default.

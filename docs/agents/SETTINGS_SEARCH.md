# Preferences search

Preferences search uses a static catalogue of section titles, actual control
captions, descriptions and common synonyms. Search includes controls in account,
calendar, Google, profile and backup forms. It indexes no personal names,
addresses, passwords, tokens or saved configuration values.

The common matcher normalises Latin accents, preserves other scripts and requires
every distinct query word. Exact section titles come first, followed by exact
control captions, caption prefixes and weighted word matches. Title words have
more weight than control captions, tab names, descriptions or synonyms. The
selected matcher uses Nucleo abbreviations with RapidFuzz OSA correction,
including adjacent transpositions. A query word spelled like a real catalogue
word, or a prefix of one, is not also treated as a typo of a different word, so
"shared profile" finds Profiles and sync rather than the local Profiles card
through "saved". Words with no catalogue spelling keep typo correction.
Numeric words match exactly. Stable title ordering resolves ties. `matches_with`
retains the pure word-scoring boundary. The word matcher does not bridge
abbreviations across separate caption words.

Static text is normalised once. Results are calculated when the input changes
and reused during drawing and observations. The ignored ranking measurement
checks complete warmed searches separately from native input and rendering;
run it only after other builds stop.

Each result is one section, ranked as above. Within it, every control caption is
scored with the same word matcher, counting only exact, prefix and abbreviation
matches; a typo correction finds the section but never names a control. The
caption matching the most query words wins, then the one whose words read as the
query in order, then the lowest score, then the shorter caption. It is named when
it matches at least half of the query words and more of them than the section
title does, so "shared profile" names Check for shared profiles after Google
sign-in and "new mail interval" names Check for new mail, while "backup",
"profile workspace" and the synonym "retention" open their sections at the top.

Selecting a named result opens the section, then reveals the control. The reveal
is a widget operation over actual layout: it finds the caption's rendered text
inside the Preferences scroller (`settings-scroll`), scrolls only when the
caption is not fully visible and focuses a text field on the caption's row or
directly below it with no other caption between. Checkboxes, buttons and pick
lists are scrolled to but not focused. A caption that is not laid out after about
250 ms, such as Retry Google cleanup while Google is connected, leaves the
section open at its top. Newer navigation or a new query discards a pending
reveal. Selecting a result never changes a preference.

The revealed control gets a 2 px accent outline with a faint tint: a labelled
text field with its label, otherwise the caption's own button or row (the
innermost enclosing container that adds only padding) or the caption itself, as
for a checkbox inside a card. The outline is a second `stack` layer inside the
scroller, so it scrolls with the content, draws above it and never captures
input. It clears after 1.8 seconds, or on the next click, key press or wheel
scroll. The layer is always present, as an empty space when idle, so the
Preferences widget tree and its focus survive the outline appearing.

Coverage is enforced by tests. `coverage.rs` renders every Preferences tab in
empty, connected, enrolled, disconnected-Google and several backup states,
collects every caption the scroller lays out, and fails when one is not a
catalogue title or control, a whole-word phrase in a description, or a
documented data, count, binding or sample caption. The same test fails when a
catalogue control is never laid out, unless it is listed with the state that
shows it. Form fields in dialogs and pick-list options live in descriptions,
because they are searchable but cannot be revealed in place.
Tests also exhaustively destructure `Preferences`, compare its serialised keys
with search coverage and documented exclusions, and audit nested
network/configuration schemas. Excluded fields are layout/sidebar state, mailbox
sorting, per-message image decisions, provider receipts, credential identifiers,
legacy OAuth data and the old calendar interval. Password controls are indexed by
caption only.

Browser and Flutter clients have no Preferences search yet. Those parity gaps
remain open in TODO and `docs/CLIENT_PARITY.md`; the desktop catalogue does not
establish client parity.

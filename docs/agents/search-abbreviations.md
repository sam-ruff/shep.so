# Abbreviation search demo

`demo/search-abbreviations` compares Nucleo abbreviation matching with OSA typo
correction for Preferences and Move, plus phrase priority for indexed mail search.
This branch is a desktop alternative for review, not a chosen production default.

Use a separate checkout and a disposable profile to compare the demos:

```sh
git worktree add ../shep-search-abbreviations origin/demo/search-abbreviations
cd ../shep-search-abbreviations
demo_home=$(mktemp -d)
XDG_DATA_HOME="$demo_home/data" XDG_CONFIG_HOME="$demo_home/config" XDG_CACHE_HOME="$demo_home/cache" CARGO_BUILD_JOBS=4 cargo run --profile test-ui --features test-support -- --demo --search-mail --nested-folders
```

In Preferences, try `sys tr`, `sfp fng`, `dark mode` and `appearnce`. In Move,
compare `Archive`, `archvie` and `pjarch` against folders named `Archive` and
`Projects/Archive`. Mail needs cached messages to compare: a short body equal to
the query comes first; a matching phrase then precedes separated exact words,
which precede prefix or corrected words. For example, search `architecture plan
17` among messages containing that phrase, `17 plan architecture` and
`architecture planning 17`. The number `170` does not match `17`.

Exact labels, exact folder leaves and prefixes receive the strongest label
scores. Every remaining query term must match. Nucleo rewards compact
subsequences and word boundaries; if a term has no subsequence match, OSA allows
one edit for words of at least three characters and two for at least six.
Transpositions count as one edit. Numeric terms require a complete token.
Punctuation is passed literally to the low-level matcher, so `!`, `^`, `$` and
`|` do not become query operators. One matcher and its scratch buffer are reused
through each result batch.

Mail uses SQLite FTS5 and field-weighted BM25 within each tier. Subject, body and
sender weights are 2.0, 1.0 and 0.3. Phrase and literal relations are computed
once, with a shared relation for single-term queries. The list and frozen bulk
selection use identical tier, score, timestamp and identity ordering; pages
remain limited to 50 metadata rows.

Mail abbreviations do not scan every message or vocabulary term. Mail keeps the
bounded existing expansion: up to 12 query tokens, 32 one-edit vocabulary
candidates, 256 prefix-neighbour candidates for longer words and 12 corrections
per token. Only ASCII alphabetic words receive typo expansion; Unicode literal
matching remains available. Latin accents are normalised without stripping
Japanese marks or decomposing Hangul. BM25 statistics use the cached collection,
while account, folder and flag filters still restrict returned results.

Algorithm references: [Nucleo matcher](https://docs.rs/nucleo-matcher/0.3.1/nucleo_matcher/),
[RapidFuzz OSA](https://docs.rs/rapidfuzz/latest/rapidfuzz/distance/osa/index.html)
and [SQLite FTS5](https://www.sqlite.org/fts5.html).

Automated ranking and capture coverage lives in `tests/search_abbreviations.rs`,
alongside the shared search and selection suites. Native demonstrations and
performance receipts are recorded in the completion log after review. Browser
and mobile do not acquire this experimental matcher by switching desktop branches;
their search parity remains tracked separately.

# Abbreviation search demo

`demo/search-abbreviations` compares Nucleo abbreviation matching with OSA typo
correction for Preferences and Move, plus phrase priority for indexed mail search.
This branch is a desktop alternative for review, not a chosen production default.

Use a separate checkout and a disposable profile to compare the demos:

```sh
git worktree add ../shep-search-abbreviations origin/demo/search-abbreviations
cd ../shep-search-abbreviations
shep_demo_dir=$(mktemp -d "${TMPDIR:-/tmp}/shep-search-demo.XXXXXX")
CARGO_BUILD_JOBS=4 cargo run --profile test-ui --features test-support -- --demo --search-mail --persist-demo --test-state "$shep_demo_dir/state.json"
```

Use a fresh directory for each branch. Reusing the directory retains changes to
the fictional workspace and allows the Profiles controls to open.

In Preferences, try `prf`, `ntfctns`, `dark mode` and `appearnce`. In Move,
compare `Archive`, `archvie` and `pjarch` against folders named `Archive` and
`Projects/Archive`. Mail needs cached messages to compare: a short body equal to
the query comes first; a matching phrase then precedes separated exact words,
which precede prefix or corrected words. For example, search `architecture plan
17` among messages containing that phrase, `17 plan architecture` and
`architecture planning 17`. The number `170` does not match `17`.

Exact labels, exact folder leaves and prefixes receive the strongest label
scores. Every remaining query term must match. Nucleo rewards compact
subsequences and word boundaries; if a term has no subsequence match, OSA allows
one edit for words of at least four characters in Settings or three in Move,
and two for at least six.
Transpositions count as one edit. Label terms containing digits require a
complete token, so `S3` cannot match `S30`.
Folder punctuation is passed literally to the low-level matcher; Preferences
and mail split text into words. Neither enables `!`, `^`, `$` or `|` as query
operators. One matcher and its scratch buffer are reused through each result
batch. Preferences caches each query word's characters alongside the immutable
catalogue words and their field weights.
Preferences abbreviations match individual indexed words; folder abbreviations
can span path segments, as in `pjarch` for `Projects/Archive`.

Mail uses SQLite FTS5 and field-weighted BM25 within each tier. Subject, body and
sender weights are 2.0, 1.0 and 0.3. Phrase and literal relations are computed
once, with a shared relation for single-term queries. The list and frozen bulk
selection use identical tier, score, timestamp and identity ordering; pages
remain limited to 50 metadata rows.

Search totals and unread counts are computed alongside the ranked keys inside
SQLite, then at most 50 metadata rows are read. An empty page still retrieves
the complete counts. Queries containing only proven exact terms share their
literal ranking instead of computing it again; phrase priority remains intact.
Unicode exact-body matches retain their priority even when SQLite's tokeniser
and the query normaliser differ for combined accents.
For ASCII alphabetic words, the current vocabulary is checked inside the query
transaction. If no longer term exists, an exact FTS lookup avoids unnecessary
prefix merging. The proof is repeated for each query, so new word extensions
remain searchable after insertion, removal and reopening the cache.
When a bounded prefix scan exhausts its range, impossible spelling variants in
that range are skipped. Other prefixes and scans that reach their cap retain
their complete candidate checks.

Mail abbreviations do not scan every message or vocabulary term. Mail keeps the
bounded expansion: the first 12 query tokens, 32 one-edit vocabulary
candidates, 256 prefix-neighbour candidates for longer words and 12 corrections
per token. Both candidate budgets count typo neighbours and exclude the exact
token, which already has its own literal/prefix alternative. This can admit one
more neighbour at the earlier cap boundary and avoids recounting the exact
word's postings during vocabulary lookup. Further query tokens are ignored.
Only ASCII alphabetic words receive typo expansion; Unicode literal
matching remains available. Wholly numeric mail tokens are exact; mixed letters
and digits retain prefix matching, so mail `S3` can still match `S30`. Latin accents are normalised without stripping
Japanese marks or decomposing Hangul. BM25 statistics use the cached collection,
while account, folder and flag filters still restrict returned results.

Algorithm references: [Nucleo matcher](https://docs.rs/nucleo-matcher/0.3.1/nucleo_matcher/),
[RapidFuzz OSA](https://docs.rs/rapidfuzz/latest/rapidfuzz/distance/osa/index.html)
and [SQLite FTS5](https://www.sqlite.org/fts5.html).

Automated ranking and capture coverage lives in `tests/search_abbreviations.rs`,
alongside the shared search and selection suites. Native demonstrations and
performance receipts are recorded in the completion log after review. On the
shared 100,000-message fixture, cached search p95 is 19.87 ms for ordinary text,
22.83 ms for a transposition and 31.25 ms for four terms. Warm complete
Preferences ranking is 0.023–0.045 ms p95; these measurements exclude native
input/drawing and live providers. Browser
and mobile do not acquire this experimental matcher by switching desktop branches;
their search parity remains tracked separately.

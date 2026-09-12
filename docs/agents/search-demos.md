# Search demos

Three branches compare text matching for Preferences, folders and cached mail.
They share the same settings catalogue and fictional mail, with the reader and
icon fixes from main. No alternative is the selected production default yet.

| Branch | Combination | Intended strength |
| --- | --- | --- |
| `demo/search-precise` | Exact/prefix matches, RapidFuzz OSA and BM25 | Clear words and ordinary typing mistakes |
| `demo/search-tolerant` | Damerau-Levenshtein, Jaro-Winkler and weighted BM25 | Broader typo tolerance |
| `demo/search-abbreviations` | Nucleo, OSA fallback and phrase-aware BM25 | Abbreviated option/folder names and mail phrases |

From a demo branch, open an isolated fictional workspace:

```sh
shep_demo_dir=$(mktemp -d "${TMPDIR:-/tmp}/shep-search-demo.XXXXXX")
cargo run --profile test-ui --features test-support -- \
  --demo --search-mail --persist-demo --test-state "$shep_demo_dir/state.json"
```

This does not use personal accounts or change the installed application.
Run both lines for each branch to start fresh. Reusing the directory preserves
that demo's preferences, mail changes and profiles.
The comparison replaces twelve generic fixture rows, preserving their cache
identities and row counts. The original exact `test` examples remain present.

Try these in mail Search, keeping **Best match** selected:

- `test`: the short **Quick note** should come first.
- `project review`: **Roadmap agenda**, whose entire body is that phrase, comes first.
- `release planning`: compare the phrase with **Planning the release**.
- `confernece`: find **Conference booking** despite swapped letters.
- `cnfernece`: compare a missing letter plus swapped letters.
- `invoice 2026`: the numeric identifier must not match **Invoice 2027**.
- `cafe`: find **Café reservation** without an accent.

In Move, try `archvie`, `cafe` and `pjarch`; Escape cancels without moving mail.
In Preferences, compare `dark mode`, `apperance`, `retention`, `sftp fingerprint`
and `synced passwords`. Results open their actual settings sections. Deep links
to individual controls inside long, state-dependent cards remain R99 work.

Mail retains the existing parser in all three demos: it uses the first twelve
tokens, with AND between those tokens. Further tokens are currently ignored.
Wholly numeric tokens such as `2026` match exactly, while mixed terms such as
`S3` retain prefix matching and can match `S30`. These limits remain tracked;
option/folder labels use stricter digit-bearing token matching.

The saved native scenario is
`test_search_comparison_exact_phrase_typo_numbers_and_folder_abbreviation` in
`scripts/e2e.py`. Each algorithm must also pass the common search and captured
selection tests. Record measured timings and ranking limits with each branch;
the intended strengths above are not benchmark results.

The common fixture passes its three native search/Move scenarios, including
the existing exact-body ranking and accent/fast-Enter checks. Reviewed light
and compact-dark captures are under `artifacts/e2e/156acd787b79`, with the run
in `artifacts/logs/search-fixture-native.log`. This verifies the comparison data
and controls before changing algorithms, not the proposed alternatives.

Algorithm references: [RapidFuzz](https://docs.rs/rapidfuzz/latest/rapidfuzz/),
[Nucleo](https://docs.rs/nucleo-matcher/latest/nucleo_matcher/), and
[SQLite FTS5](https://www.sqlite.org/fts5.html).

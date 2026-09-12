# Search comparison

Three desktop branches compare ranked search in Preferences, Move and cached
mail. Each includes the formatted HTML selection, newest-first conversations,
pinned reply actions and icon fixes from main. No search alternative has been
selected for main.

| Demo branch | Combination | Try it for |
| --- | --- | --- |
| [Precise](https://github.com/sam-ruff/shep.so/tree/demo/search-precise) | Exact/prefix matching, RapidFuzz OSA and BM25 | Clear words and ordinary typing mistakes |
| [Tolerant](https://github.com/sam-ruff/shep.so/tree/demo/search-tolerant) | Damerau-Levenshtein, Jaro-Winkler and weighted BM25 | Harder typing mistakes |
| [Abbreviations](https://github.com/sam-ruff/shep.so/tree/demo/search-abbreviations) | Nucleo, OSA fallback and phrase-aware BM25 | Abbreviated settings/folders and mail phrases |

From any demo branch, run:

```sh
shep_demo_dir=$(mktemp -d "${TMPDIR:-/tmp}/shep-search-demo.XXXXXX")
cargo run --profile test-ui --features test-support -- \
  --demo --search-mail --persist-demo --test-state "$shep_demo_dir/state.json"
```

This opens the same fictional mail in a fresh temporary workspace, including
working profile preferences. It does not use personal accounts or replace the
installed application. Run both lines for each branch to compare fresh state;
reuse the directory to retain that demo's changes.

In mail Search, keep **Best match** selected. Try `test` and `project review`
for complete-body matches, `release planning` for phrase order, `confernece`
for a transposition and `cnfernece` for a harder typo. `invoice 2026` keeps the
year exact; `cafe` finds Café without an accent.

In Preferences, try `dark mode`, `apperance`, `retention`, `sftp fingerprint`
and `synced passwords`. Compare `ntfctns` and `prf` on the abbreviation branch.
In Move, try `archvie`, `cafe` and `pjarch`; Escape cancels without moving mail.

The local mailbox benchmark uses 100,000 cached messages across four accounts,
60 samples per query and at most 50 returned rows. The searches below each
match 4,132 messages across sender, subject and body fields.

| Query p95 | Precise | Tolerant | Abbreviations |
| --- | ---: | ---: | ---: |
| `milestone 17` | 21.57 ms | 20.20 ms | 19.87 ms |
| `milestnoe 17` | 24.81 ms | 24.26 ms | 22.83 ms |
| `architecture plans milestone 17` | 33.04 ms | 32.07 ms | 31.25 ms |

All pass the existing 50 ms page budget. Settings catalogue ranking stayed
below 0.21 ms at p95 across eight queries. Builds and native tests were stopped
during timing; these measurements cover the specified cached queries and
ranking work, not full input-to-screen latency or every mailbox shape.

All alternatives share the expanded settings catalogue and visible-label ranking.
Results open settings sections. Revealing an individual control inside a long,
state-dependent section remains R99 work, along with complete dynamic-caption
coverage and mobile/browser parity.

Mail retains its existing parser: only the first twelve tokens are used, with
AND between them; further tokens are ignored. Wholly numeric words match
exactly, while mixed terms such as `S3` can prefix-match `S30`. Settings and Move
use stricter digit-bearing token matching. Typo candidate limits also remain
bounded and can omit a possible match. These limits need review before a
production matcher is selected.

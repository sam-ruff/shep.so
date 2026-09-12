# Search matching

Sam selected **Abbreviations** on 12 September after trying all three options
with his actual profiles. Desktop search now uses Nucleo for abbreviated
Preferences and folder labels, OSA typo correction and phrase-aware BM25 mail
ranking. See [matching details](search-abbreviations.md).

In Preferences, try `ntfctns`, `prf`, `dark mode` or `sftp fingerprint`.
In Move, try an abbreviated folder name or a transposition. Mail search keeps
exact short-body matches first, then phrases, literal terms and corrected terms.
Use **Best match** to see relevance ordering.

To explore fictional mail in a separate workspace:

```sh
shep_demo_dir=$(mktemp -d "${TMPDIR:-/tmp}/shep-search-demo.XXXXXX")
cargo run --profile test-ui --features test-support -- \
  --demo --search-mail --persist-demo --test-state "$shep_demo_dir/state.json"
```

Run without fixture flags to use the normal saved profiles. Reusing the temporary
directory preserves that fictional workspace's settings and mail changes.

## Comparison evidence

Precise and Tolerant were not selected. Their branch references are retired;
the [completion audit](../COMPLETION.md) retains their commits and verification.

| Option | Combination | Multiword query p95 |
| --- | --- | ---: |
| Precise | Exact/prefix, RapidFuzz OSA, BM25 | 33.04 ms |
| Tolerant | Damerau-Levenshtein, Jaro-Winkler, weighted BM25 | 32.07 ms |
| Abbreviations, selected | Nucleo, OSA, phrase-aware BM25 | 31.25 ms |

The local benchmark used 100,000 cached messages across four accounts, 60
samples per query and at most 50 returned rows. The multiword query matched
4,132 messages. All passed the existing 50 ms budget. These are cached-query
measurements, not complete input-to-screen latency or every mailbox shape.

Results currently open settings sections. Individual-control reveal, complete
dynamic captions and Flutter/browser parity remain open. Mail uses only its
first twelve tokens, with AND between them; wholly numeric tokens stay exact,
while mixed identifiers such as `S3` can prefix-match `S30`. Typo discovery is
bounded and can omit a possible correction.

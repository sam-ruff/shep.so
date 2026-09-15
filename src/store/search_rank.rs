//! Tiered BM25 inside one FTS5 pass. The MATCH expression carries the phrase,
//! exact and expanded groups; each row's tier and score come from the
//! instance lists FTS5 already holds for that row. Per-phrase document counts
//! arrive with the query, so ranking never rescans the index once per phrase
//! the way the built-in `bm25()` does.
use anyhow::Context;
use rusqlite::{Connection, ffi};
use std::ffi::{CStr, c_int, c_void};
use std::ptr;

const K1: f64 = 1.2;
const B: f64 = 0.75;
const MIN_IDF: f64 = 1e-6;
/// A negative document count means the caller could not supply one and FTS5
/// counts the phrase itself.
#[cfg(test)]
const UNKNOWN_DOCS: i64 = -1;

/// `phrase` is the document count of the first group's phrases read as one
/// adjacent phrase, which then forms the top tier; `ends` are cumulative
/// phrase end indexes per group in FTS5 phrase order; `docs` holds one
/// document count per phrase.
pub(super) fn spec(phrase: Option<i64>, ends: &[usize], docs: &[i64]) -> String {
    let join = |values: &mut dyn Iterator<Item = String>| values.collect::<Vec<_>>().join(",");
    format!(
        "{}|{}|{}",
        phrase.map_or_else(|| "-".to_owned(), |docs| docs.to_string()),
        join(&mut ends.iter().map(usize::to_string)),
        join(&mut docs.iter().map(i64::to_string))
    )
}

struct Spec {
    phrase: Option<i64>,
    ends: Vec<usize>,
    docs: Vec<i64>,
}

impl Spec {
    fn parse(text: &str) -> Option<Self> {
        let mut parts = text.splitn(3, '|');
        let (phrase, ends, docs) = (parts.next()?, parts.next()?, parts.next()?);
        let phrase = match phrase {
            "-" => None,
            docs => Some(docs.parse::<i64>().ok().filter(|docs| *docs >= 0)?),
        };
        let ends = ends
            .split(',')
            .map(str::parse)
            .collect::<Result<Vec<usize>, _>>()
            .ok()?;
        let docs = docs
            .split(',')
            .map(str::parse)
            .collect::<Result<Vec<i64>, _>>()
            .ok()?;
        let consistent = ends.windows(2).all(|pair| pair[0] <= pair[1])
            && ends.last().is_some_and(|last| *last == docs.len())
            && (phrase.is_none() || ends[0] > 1);
        consistent.then_some(Self { phrase, ends, docs })
    }
}

/// The first group's phrases read as one phrase: adjacent, in order, within
/// one column, exactly as FTS5 matches a quoted phrase.
struct Phrase {
    idf: f64,
    sizes: Vec<c_int>,
}

/// Per-query state FTS5 keeps on the cursor for the function that built it.
struct Ranking {
    phrase: Option<Phrase>,
    ends: Vec<usize>,
    idf: Vec<f64>,
    avgdl: f64,
    freq: Vec<f64>,
    hits: Vec<u32>,
    instances: Vec<(usize, c_int, c_int, f64)>,
}

impl Ranking {
    /// Weighted count of the first group's phrases occurring as one phrase.
    fn phrase_frequency(&self, phrase: &Phrase) -> f64 {
        let mut frequency = 0.0;
        for &(first, column, offset, weight) in &self.instances {
            if first != 0 {
                continue;
            }
            let mut next = offset + phrase.sizes[0];
            let adjacent = phrase.sizes[1..].iter().enumerate().all(|(index, size)| {
                let found = self.instances.iter().any(
                    |&(instance, instance_column, instance_offset, _)| {
                        instance == index + 1
                            && instance_column == column
                            && instance_offset == next
                    },
                );
                next += size;
                found
            });
            if adjacent {
                frequency += weight;
            }
        }
        frequency
    }
}

fn bm25_term(idf: f64, freq: f64, length: f64, avgdl: f64) -> f64 {
    idf * (freq * (K1 + 1.0)) / (freq + K1 * (1.0 - B + B * length / avgdl))
}

pub(super) fn register(connection: &Connection) -> anyhow::Result<()> {
    // SAFETY: the handle belongs to an open connection and FTS5 keeps its own
    // copy of the function pointers, which are plain `extern "C"` items.
    unsafe {
        let api = fts5_api(connection.handle())?;
        let create = (*api)
            .xCreateFunction
            .context("FTS5 cannot register functions")?;
        for (name, function) in [
            (c"shep_search_tier", tier_function as Function),
            (c"shep_search_score", score_function as Function),
        ] {
            let rc = create(api, name.as_ptr(), ptr::null_mut(), Some(function), None);
            anyhow::ensure!(
                rc == ffi::SQLITE_OK,
                "Could not register {} ({rc})",
                name.to_string_lossy()
            );
        }
    }
    Ok(())
}

type Function = unsafe extern "C" fn(
    *const ffi::Fts5ExtensionApi,
    *mut ffi::Fts5Context,
    *mut ffi::sqlite3_context,
    c_int,
    *mut *mut ffi::sqlite3_value,
);

unsafe fn fts5_api(db: *mut ffi::sqlite3) -> anyhow::Result<*mut ffi::fts5_api> {
    let mut statement = ptr::null_mut();
    let rc = unsafe {
        ffi::sqlite3_prepare_v2(
            db,
            c"SELECT fts5(?1)".as_ptr(),
            -1,
            &mut statement,
            ptr::null_mut(),
        )
    };
    anyhow::ensure!(rc == ffi::SQLITE_OK, "FTS5 is unavailable ({rc})");
    let mut api: *mut ffi::fts5_api = ptr::null_mut();
    let rc = unsafe {
        let bound = ffi::sqlite3_bind_pointer(
            statement,
            1,
            (&mut api as *mut *mut ffi::fts5_api).cast::<c_void>(),
            c"fts5_api_ptr".as_ptr(),
            None,
        );
        let stepped = if bound == ffi::SQLITE_OK {
            ffi::sqlite3_step(statement)
        } else {
            bound
        };
        ffi::sqlite3_finalize(statement);
        stepped
    };
    anyhow::ensure!(
        rc == ffi::SQLITE_ROW && !api.is_null(),
        "FTS5 did not expose its extension API ({rc})"
    );
    Ok(api)
}

unsafe extern "C" fn tier_function(
    api: *const ffi::Fts5ExtensionApi,
    fts: *mut ffi::Fts5Context,
    context: *mut ffi::sqlite3_context,
    count: c_int,
    values: *mut *mut ffi::sqlite3_value,
) {
    // SAFETY: FTS5 hands every pointer to an auxiliary function for the
    // duration of this call only, and this call does not retain them.
    unsafe {
        match row(api, fts, count, values) {
            Ok((tier, _)) => ffi::sqlite3_result_int(context, tier as c_int),
            Err(message) => fail(context, message),
        }
    }
}

unsafe extern "C" fn score_function(
    api: *const ffi::Fts5ExtensionApi,
    fts: *mut ffi::Fts5Context,
    context: *mut ffi::sqlite3_context,
    count: c_int,
    values: *mut *mut ffi::sqlite3_value,
) {
    // SAFETY: as for `tier_function`.
    unsafe {
        match row(api, fts, count, values) {
            Ok((_, score)) => ffi::sqlite3_result_double(context, score),
            Err(message) => fail(context, message),
        }
    }
}

unsafe fn fail(context: *mut ffi::sqlite3_context, message: &'static str) {
    unsafe { ffi::sqlite3_result_error(context, message.as_ptr().cast(), message.len() as c_int) };
}

unsafe extern "C" fn drop_ranking(ranking: *mut c_void) {
    unsafe { drop(Box::from_raw(ranking.cast::<Ranking>())) };
}

unsafe extern "C" fn count_row(
    _: *const ffi::Fts5ExtensionApi,
    _: *mut ffi::Fts5Context,
    total: *mut c_void,
) -> c_int {
    unsafe { *total.cast::<i64>() += 1 };
    ffi::SQLITE_OK
}

/// The row's tier and the BM25 score of that tier's phrases, weighted by the
/// per-column arguments after the spec like `bm25()`.
unsafe fn row(
    api: *const ffi::Fts5ExtensionApi,
    fts: *mut ffi::Fts5Context,
    count: c_int,
    values: *mut *mut ffi::sqlite3_value,
) -> Result<(usize, f64), &'static str> {
    if api.is_null() || fts.is_null() || values.is_null() || count < 1 {
        return Err("shep_search functions need a ranking spec");
    }
    let api = unsafe { &*api };
    let ranking = unsafe { ranking(api, fts, *values)? };
    ranking.freq.fill(0.0);
    ranking.hits.fill(0);
    ranking.instances.clear();
    let inst_count = api.xInstCount.ok_or("FTS5 has no xInstCount")?;
    let inst = api.xInst.ok_or("FTS5 has no xInst")?;
    let mut instances: c_int = 0;
    if unsafe { inst_count(fts, &mut instances) } != ffi::SQLITE_OK {
        return Err("FTS5 could not list phrase instances");
    }
    for index in 0..instances {
        let (mut phrase, mut column, mut offset): (c_int, c_int, c_int) = (0, 0, 0);
        if unsafe { inst(fts, index, &mut phrase, &mut column, &mut offset) } != ffi::SQLITE_OK {
            return Err("FTS5 could not read a phrase instance");
        }
        let weight = if column + 1 < count {
            unsafe { ffi::sqlite3_value_double(*values.add(column as usize + 1)) }
        } else {
            1.0
        };
        let phrase = usize::try_from(phrase).map_err(|_| "FTS5 reported a negative phrase")?;
        let (Some(freq), Some(hits)) = (ranking.freq.get_mut(phrase), ranking.hits.get_mut(phrase))
        else {
            return Err("FTS5 reported a phrase outside the ranking spec");
        };
        *freq += weight;
        *hits += 1;
        if ranking.phrase.is_some() && phrase < ranking.ends[0] {
            ranking.instances.push((phrase, column, offset, weight));
        }
    }
    let column_size = api.xColumnSize.ok_or("FTS5 has no xColumnSize")?;
    let mut tokens: c_int = 0;
    if unsafe { column_size(fts, -1, &mut tokens) } != ffi::SQLITE_OK {
        return Err("FTS5 could not read the row size");
    }
    let length = f64::from(tokens);
    // Earlier groups are conjunctions of their phrases. The last group is the
    // expression's own fallback: a matched row always belongs to it.
    let mut start = 0;
    let mut group = ranking.ends.len().saturating_sub(1);
    for (index, &end) in ranking.ends.iter().enumerate() {
        if index + 1 == ranking.ends.len() {
            break;
        }
        if ranking.hits[start..end].iter().all(|hits| *hits > 0) {
            group = index;
            break;
        }
        start = end;
    }
    let end = ranking.ends.get(group).copied().unwrap_or(start);
    let Some(phrase) = &ranking.phrase else {
        let score: f64 = (start..end)
            .map(|index| {
                bm25_term(
                    ranking.idf[index],
                    ranking.freq[index],
                    length,
                    ranking.avgdl,
                )
            })
            .sum();
        return Ok((group, -score));
    };
    // The whole phrase outranks its separated words; both sit above the
    // expanded alternatives.
    if group == 0 {
        let frequency = ranking.phrase_frequency(phrase);
        if frequency > 0.0 {
            let score = bm25_term(phrase.idf, frequency, length, ranking.avgdl);
            return Ok((0, -score));
        }
    }
    let score: f64 = (start..end)
        .map(|index| {
            bm25_term(
                ranking.idf[index],
                ranking.freq[index],
                length,
                ranking.avgdl,
            )
        })
        .sum();
    Ok((group + 1, -score))
}

unsafe fn ranking<'a>(
    api: &ffi::Fts5ExtensionApi,
    fts: *mut ffi::Fts5Context,
    spec: *mut ffi::sqlite3_value,
) -> Result<&'a mut Ranking, &'static str> {
    let get = api.xGetAuxdata.ok_or("FTS5 has no xGetAuxdata")?;
    let existing = unsafe { get(fts, 0) }.cast::<Ranking>();
    if !existing.is_null() {
        return Ok(unsafe { &mut *existing });
    }
    let text = unsafe { ffi::sqlite3_value_text(spec) };
    if text.is_null() {
        return Err("shep_search functions need a text ranking spec");
    }
    let text = unsafe { CStr::from_ptr(text.cast()) }
        .to_str()
        .map_err(|_| "The ranking spec is not UTF-8")?;
    let Spec { phrase, ends, docs } = Spec::parse(text).ok_or("The ranking spec is malformed")?;
    let phrase_count = api.xPhraseCount.ok_or("FTS5 has no xPhraseCount")?;
    let phrases = usize::try_from(unsafe { phrase_count(fts) }).unwrap_or(0);
    if phrases != docs.len() {
        return Err("The ranking spec does not match the query's phrases");
    }
    let phrase_size = api.xPhraseSize.ok_or("FTS5 has no xPhraseSize")?;
    let sizes: Vec<c_int> = (0..ends[0])
        .map(|index| unsafe { phrase_size(fts, index as c_int) })
        .collect();
    let row_count = api.xRowCount.ok_or("FTS5 has no xRowCount")?;
    let total_size = api.xColumnTotalSize.ok_or("FTS5 has no xColumnTotalSize")?;
    let query_phrase = api.xQueryPhrase.ok_or("FTS5 has no xQueryPhrase")?;
    let (mut rows, mut tokens): (ffi::sqlite3_int64, ffi::sqlite3_int64) = (0, 0);
    if unsafe { row_count(fts, &mut rows) } != ffi::SQLITE_OK
        || unsafe { total_size(fts, -1, &mut tokens) } != ffi::SQLITE_OK
    {
        return Err("FTS5 could not read its collection statistics");
    }
    let inverse_frequency = |hits: i64| {
        let value = ((rows as f64 - hits as f64 + 0.5) / (hits as f64 + 0.5)).ln();
        if value <= 0.0 { MIN_IDF } else { value }
    };
    let mut idf = Vec::with_capacity(phrases);
    for (index, &known) in docs.iter().enumerate() {
        let mut hits = known;
        if hits < 0 {
            hits = 0;
            let counted = unsafe {
                query_phrase(
                    fts,
                    index as c_int,
                    (&mut hits as *mut i64).cast::<c_void>(),
                    Some(count_row),
                )
            };
            if counted != ffi::SQLITE_OK {
                return Err("FTS5 could not count a phrase");
            }
        }
        idf.push(inverse_frequency(hits));
    }
    let ranking = Box::into_raw(Box::new(Ranking {
        phrase: phrase.map(|docs| Phrase {
            idf: inverse_frequency(docs),
            sizes,
        }),
        ends,
        idf,
        avgdl: tokens as f64 / rows as f64,
        freq: vec![0.0; phrases],
        hits: vec![0; phrases],
        instances: Vec::new(),
    }));
    let set = api.xSetAuxdata.ok_or("FTS5 has no xSetAuxdata")?;
    if unsafe { set(fts, ranking.cast::<c_void>(), Some(drop_ranking)) } != ffi::SQLITE_OK {
        unsafe { drop(Box::from_raw(ranking)) };
        return Err("FTS5 could not keep the ranking state");
    }
    Ok(unsafe { &mut *ranking })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        register(&connection).unwrap();
        connection
            .execute_batch(
                "CREATE VIRTUAL TABLE notes USING fts5(sender, subject, body);
                 INSERT INTO notes VALUES('Alex', 'Plan', 'architecture plan for seventeen');
                 INSERT INTO notes VALUES('Sam', 'Architecture plan', 'plan plan plan');
                 INSERT INTO notes VALUES('Plan Person', 'Notes', 'architecture only');
                 INSERT INTO notes VALUES('Kim', 'Other', 'unrelated words here');",
            )
            .unwrap();
        connection
    }

    fn scores(c: &Connection, sql: &str, params: &[&str]) -> Vec<(i64, f64)> {
        c.prepare(sql)
            .unwrap()
            .query_map(rusqlite::params_from_iter(params), |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    fn docs(c: &Connection, phrase: &str) -> i64 {
        c.query_row(
            "SELECT COUNT(*) FROM notes WHERE notes MATCH ?",
            [phrase],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn spec_round_trips_and_rejects_inconsistent_text() {
        let text = spec(Some(7), &[2, 3], &[4, 100, UNKNOWN_DOCS]);
        assert_eq!(text, "7|2,3|4,100,-1");
        let parsed = Spec::parse(&text).unwrap();
        assert_eq!(
            (parsed.phrase, parsed.ends, parsed.docs),
            (Some(7), vec![2, 3], vec![4, 100, -1])
        );
        let parsed = Spec::parse(&spec(None, &[1], &[4])).unwrap();
        assert_eq!((parsed.phrase, parsed.ends), (None, vec![1]));
        for bad in [
            "",
            "-|1,3",
            "-|1,3|4",
            "-|3,1|4,5,6",
            "-|a|1",
            "-|1|x",
            "x|2|1,2",
            "-1|2|1,2",
            "4|1|1",
        ] {
            assert!(Spec::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn single_group_scores_equal_the_built_in_bm25() {
        let c = connection();
        let literal = "\"architecture\" AND \"plan\"";
        let expected = scores(
            &c,
            "SELECT rowid,bm25(notes,0.3,2.0,1.0) FROM notes WHERE notes MATCH ? ORDER BY rowid",
            &[literal],
        );
        let spec = spec(
            None,
            &[2],
            &[docs(&c, "\"architecture\""), docs(&c, "\"plan\"")],
        );
        let actual = scores(
            &c,
            "SELECT rowid,shep_search_score(notes,?,0.3,2.0,1.0) FROM notes WHERE notes MATCH ? ORDER BY rowid",
            &[&spec, literal],
        );
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), 3);
        let tiers = scores(
            &c,
            "SELECT rowid,shep_search_tier(notes,?) FROM notes WHERE notes MATCH ? ORDER BY rowid",
            &[&spec, literal],
        );
        assert!(tiers.iter().all(|(_, tier)| *tier == 0.0));
    }

    #[test]
    fn unknown_counts_fall_back_to_counting_inside_fts5() {
        let c = connection();
        let literal = "\"architecture\" AND \"plan\"";
        let known = spec(
            None,
            &[2],
            &[docs(&c, "\"architecture\""), docs(&c, "\"plan\"")],
        );
        let unknown = spec(None, &[2], &[UNKNOWN_DOCS, UNKNOWN_DOCS]);
        let sql = "SELECT rowid,shep_search_score(notes,?,0.3,2.0,1.0) FROM notes WHERE notes MATCH ? ORDER BY rowid";
        assert_eq!(
            scores(&c, sql, &[&known, literal]),
            scores(&c, sql, &[&unknown, literal])
        );
    }

    #[test]
    fn tiers_score_each_row_by_its_own_group_like_separate_queries() {
        let c = connection();
        let phrase = "\"architecture plan\"";
        let literal = "\"architecture\" AND \"plan\"";
        let expanded = "(\"architecture\" OR \"architectural\") AND (\"plan\" OR \"plans\")";
        let combined = format!("({literal}) OR ({expanded})");
        let spec = spec(
            Some(docs(&c, phrase)),
            &[2, 6],
            &[
                docs(&c, "\"architecture\""),
                docs(&c, "\"plan\""),
                docs(&c, "\"architecture\""),
                docs(&c, "\"architectural\""),
                docs(&c, "\"plan\""),
                docs(&c, "\"plans\""),
            ],
        );
        let ranked = c
            .prepare("SELECT rowid,shep_search_tier(notes,?1),shep_search_score(notes,?1,0.3,2.0,1.0) FROM notes WHERE notes MATCH ?2 ORDER BY rowid")
            .unwrap()
            .query_map(rusqlite::params![spec, combined], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, f64>(2)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let phrase_scores = scores(
            &c,
            "SELECT rowid,bm25(notes,0.3,2.0,1.0) FROM notes WHERE notes MATCH ? ORDER BY rowid",
            &[phrase],
        );
        let literal_scores = scores(
            &c,
            "SELECT rowid,bm25(notes,0.3,2.0,1.0) FROM notes WHERE notes MATCH ? ORDER BY rowid",
            &[literal],
        );
        assert_eq!(
            ranked
                .iter()
                .map(|(rowid, tier, _)| (*rowid, *tier))
                .collect::<Vec<_>>(),
            [(1, 0), (2, 0), (3, 1)],
            "{ranked:?}"
        );
        for (rowid, tier, score) in &ranked {
            let expected = if *tier == 0 {
                &phrase_scores
            } else {
                &literal_scores
            };
            let (_, wanted) = expected.iter().find(|(id, _)| id == rowid).unwrap();
            assert_eq!(score, wanted, "row {rowid}");
        }
        let no_phrase = "\"plan\" AND \"architecture\"";
        let repeated = super::spec(
            None,
            &[2, 4],
            &[
                docs(&c, "\"plan\""),
                docs(&c, "\"architecture\""),
                docs(&c, "\"plan\""),
                docs(&c, "\"architecture\""),
            ],
        );
        let combined = format!("({no_phrase}) OR ({no_phrase})");
        let tiers = scores(
            &c,
            "SELECT rowid,shep_search_tier(notes,?) FROM notes WHERE notes MATCH ? ORDER BY rowid",
            &[&repeated, &combined],
        );
        assert_eq!(tiers, [(1, 0.0), (2, 0.0), (3, 0.0)]);
    }

    #[test]
    fn overlapping_repeated_words_count_phrase_instances_like_fts5() {
        let c = connection();
        let phrase = "\"plan plan\"";
        let literal = "\"plan\" AND \"plan\"";
        let spec = spec(
            Some(docs(&c, phrase)),
            &[2],
            &[docs(&c, "\"plan\""), docs(&c, "\"plan\"")],
        );
        let ranked = c
            .prepare("SELECT rowid,shep_search_tier(notes,?1),shep_search_score(notes,?1,0.3,2.0,1.0) FROM notes WHERE notes MATCH ?2 ORDER BY rowid")
            .unwrap()
            .query_map(rusqlite::params![spec, literal], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, f64>(2)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let expected = scores(
            &c,
            "SELECT rowid,bm25(notes,0.3,2.0,1.0) FROM notes WHERE notes MATCH ? ORDER BY rowid",
            &[phrase],
        );
        let phrase_rows: Vec<(i64, f64)> = ranked
            .iter()
            .filter(|(_, tier, _)| *tier == 0)
            .map(|(rowid, _, score)| (*rowid, *score))
            .collect();
        assert_eq!(phrase_rows, expected);
        assert_eq!(phrase_rows, [(2, expected[0].1)], "only plan plan plan");
        assert!(
            ranked.len() > 1,
            "single plans still match the literal group"
        );
    }

    #[test]
    fn a_spec_that_does_not_match_the_query_is_an_error() {
        let c = connection();
        let error = c
            .query_row(
                "SELECT shep_search_score(notes,?,1.0) FROM notes WHERE notes MATCH ?",
                ["-|1|4", "\"plan\" AND \"architecture\""],
                |row| row.get::<_, f64>(0),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not match"), "{error}");
        let error = c
            .query_row(
                "SELECT shep_search_score(notes,?,1.0) FROM notes WHERE notes MATCH ?",
                ["nonsense", "\"plan\""],
                |row| row.get::<_, f64>(0),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("malformed"), "{error}");
    }
}

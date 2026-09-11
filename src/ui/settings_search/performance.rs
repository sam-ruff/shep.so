use super::matches;
use std::time::Instant;

#[test]
#[ignore = "Measure only after concurrent builds stop"]
fn settings_catalogue_ranking_timings() -> anyhow::Result<()> {
    let queries = [
        ("appearance", Some("Appearance")),
        ("dark mode", Some("Appearance")),
        ("apperance", Some("Appearance")),
        ("retention", Some("Backups")),
        ("sftp fingerprint", Some("Backups")),
        ("synced passwords", Some("Profiles and sync")),
        ("sound", Some("Notifications")),
        ("qzxvjkwp", None),
    ];
    let mut metrics = Vec::new();
    for (query, expected) in queries {
        assert_eq!(
            matches(query).first().map(|setting| setting.title),
            expected
        );
        let mut samples = Vec::with_capacity(60);
        for _ in 0..60 {
            let start = Instant::now();
            let results = std::hint::black_box(matches(std::hint::black_box(query)));
            samples.push(start.elapsed().as_secs_f64() * 1000.);
            assert_eq!(results.first().map(|setting| setting.title), expected);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "{query}: p50={:.3}ms p95={:.3}ms max={:.3}ms",
            samples[30], samples[57], samples[59]
        );
        metrics.push(serde_json::json!({
            "query": query, "samples": samples.len(),
            "p50_ms": samples[30], "p95_ms": samples[57], "max_ms": samples[59],
            "first_section": expected,
        }));
    }
    std::fs::create_dir_all("artifacts/performance")?;
    std::fs::write(
        "artifacts/performance/settings-search.json",
        serde_json::to_vec_pretty(&serde_json::json!({
            "measurement": "warm complete catalogue ranking only; excludes native input and drawing",
            "metrics": &metrics,
        }))?,
    )?;
    for metric in metrics {
        let p95 = metric["p95_ms"]
            .as_f64()
            .ok_or_else(|| anyhow::anyhow!("Missing timing"))?;
        assert!(
            p95 < 8.,
            "Catalogue ranking alone exceeded the 8 ms handler budget: {metric}"
        );
    }
    Ok(())
}

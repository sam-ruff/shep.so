use sha2::{Digest, Sha256};
use shep::{model::*, store::Store};
use std::time::Instant;

fn report_directory() -> std::path::PathBuf {
    std::env::var_os("SHEP_BENCH_REPORT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "artifacts/performance".into())
}

fn context() -> serde_json::Value {
    let mode = std::env::var("SHEP_PERFORMANCE_MODE").unwrap_or_else(|_| "required".into());
    assert!(matches!(mode.as_str(), "required" | "diagnostic"));
    let executable = std::env::current_exe().unwrap();
    let binary = std::fs::read(executable).unwrap();
    let toolchain = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .unwrap();
    serde_json::json!({
        "mode": mode,
        "source": std::env::var("SHEP_PERFORMANCE_SOURCE").unwrap_or_else(|_| "unknown".into()),
        "binary_sha256": format!("{:x}", Sha256::digest(binary)),
        "toolchain": String::from_utf8(toolchain.stdout).unwrap().trim(),
    })
}

fn record_phase(name: &str, limit: f64, raw: &[f64], started_at: f64) {
    let path = report_directory().join("backend-progress.json");
    let mut report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut sorted = raw.to_vec();
    sorted.sort_by(f64::total_cmp);
    let p95 = sorted[sorted.len() * 95 / 100];
    report["phases"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "name": name, "budget_ms": limit, "p50_ms": sorted[sorted.len()/2],
            "p95_ms": p95, "raw_ms": raw, "started_at": started_at,
            "within_budget": p95 < limit,
        }));
    std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}

fn main() {
    #[cfg(feature = "test-support")]
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let report_context = context();
        std::fs::create_dir_all(report_directory()).unwrap();
        let completed_report=report_directory().join("backend.json");
        if completed_report.exists(){std::fs::remove_file(completed_report).unwrap();}
        std::fs::write(report_directory().join("backend-progress.json"),serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"dataset_messages":100000,"samples":60,"context":report_context,"phases":[]})).unwrap()).unwrap();
        let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path().join("bench.sqlite")).unwrap();
        let count=100_000;
        for batch in 0..100 {
            let messages=(0..1000).map(|i|{let i=batch*1000+i;StoredMail{summary:Mail{id:format!("bench:{i}"),account_id:format!("account-{}",i%4),remote_id:i.to_string(),folder:"INBOX".into(),sender:format!("Person {} <person@example.com>",i%100),recipient:"sam@example.com".into(),subject:format!("Project {} review",i%30),preview:"A considered message".into(),timestamp:i,unread:i%3==0,starred:i%10==0,attachment_count:0},raw:b"From: person@example.com\r\nSubject: Review\r\n\r\nA considered message".to_vec(),text:format!("Architecture plans for milestone {}",i%250)}}).collect();store.upsert(messages).await.unwrap();
        }
        async fn measure(store:&Store,query:MailQuery,name:&str,limit:f64)->f64{
            let started_at=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
            // The numeric term can occur in the sender, subject or body.
            let expected_matches=(0..100_000).filter(|i|i%100==17||i%30==17||i%250==17).count();
            let mut times=Vec::new();for _ in 0..60{let start=Instant::now();let page=store.query(query.clone()).await.unwrap();assert!(page.rows.len()<=PAGE_SIZE);if !query.search.is_empty(){assert_eq!(page.total,expected_matches,"search benchmark must return its expected matches");}times.push(start.elapsed().as_secs_f64()*1000.);}
            let raw_times=times.clone();times.sort_by(f64::total_cmp);let p95=times[times.len()*95/100];println!("{name}: p50={:.2}ms p95={p95:.2}ms budget={limit}ms",times[times.len()/2]);
            record_phase(name,limit,&raw_times,started_at);
            assert!(p95<limit,"{name} exceeded its performance budget");p95
        }
        println!("Responsiveness budget: {count} cached messages, 4 accounts, page size {PAGE_SIZE}");
        let inbox=measure(&store,MailQuery{folder:"INBOX".into(),..Default::default()},"Inbox page",50.).await;
        let account=measure(&store,MailQuery{folder:"INBOX".into(),account:Some("account-1".into()),..Default::default()},"Account page",50.).await;
        let search=measure(&store,MailQuery{search:"milestone 17".into(),sort:MailSort::Relevance,..Default::default()},"FTS search",125.).await;
        let typo=measure(&store,MailQuery{search:"milestnoe 17".into(),sort:MailSort::Relevance,..Default::default()},"Transposed search",125.).await;
        let phrase=measure(&store,MailQuery{search:"architecture plans milestone 17".into(),sort:MailSort::Relevance,..Default::default()},"Multiple-term search",150.).await;
        let body_started_at=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        let mut times=Vec::new();for i in 0..100{let start=Instant::now();store.detail(format!("bench:{i}")).await.unwrap();times.push(start.elapsed().as_secs_f64()*1000.);}
        record_phase("Cached body",10.,&times,body_started_at);
        times.sort_by(f64::total_cmp);println!("Cached body: p95={:.2}ms budget=10ms",times[95]);assert!(times[95]<10.);
        // A committed removal keeps hiding its account while cleanup is pending.
        store.run(|c|{c.execute("INSERT INTO connection_tombstones(kind,id,revision) VALUES('account','account-3',1)",[])?;Ok(())}).await.unwrap();
        assert_eq!(store.query(MailQuery{folder:"INBOX".into(),..Default::default()}).await.unwrap().total,count as usize*3/4);
        let removed=measure(&store,MailQuery{folder:"INBOX".into(),..Default::default()},"Inbox page with a removed account",50.).await;
        let report=serde_json::json!({"dataset_messages":count,"samples":60,"context":report_context,"metrics_ms":{"inbox_page_p95":inbox,"account_page_p95":account,"search_p95":search,"typo_search_p95":typo,"multiple_term_search_p95":phrase,"cached_body_p95":times[95],"removed_account_inbox_page_p95":removed}});
        std::fs::write(report_directory().join("backend.json"),serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        // A blocked network worker cannot block the UI command producer.
        let(tx,mut rx)=tokio::sync::mpsc::channel::<u64>(CHANNEL_CAPACITY);let start=Instant::now();for i in 0..CHANNEL_CAPACITY{tx.try_send(i as u64).unwrap();}assert!(tx.try_send(100).is_err());assert!(start.elapsed().as_millis()<5);assert!(rx.recv().await.is_some());
    });
}

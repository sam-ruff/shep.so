use shep::{model::*, store::Store};
use std::time::Instant;

fn main() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path().join("bench.sqlite")).unwrap();
        let count=100_000;
        for batch in 0..100 {
            let messages=(0..1000).map(|i|{let i=batch*1000+i;StoredMail{summary:Mail{id:format!("bench:{i}"),account_id:format!("account-{}",i%4),remote_id:i.to_string(),folder:"INBOX".into(),sender:format!("Person {} <person@example.com>",i%100),recipient:"sam@example.com".into(),subject:format!("Project {} review",i%30),preview:"A considered message".into(),timestamp:i,unread:i%3==0,starred:i%10==0,attachment_count:0},raw:b"From: person@example.com\r\nSubject: Review\r\n\r\nA considered message".to_vec(),text:format!("Architecture plans for milestone {}",i%250)}}).collect();store.upsert(messages).await.unwrap();
        }
        async fn measure(store:&Store,query:MailQuery,name:&str,limit:f64)->f64{
            // The numeric term can occur in the sender, subject or body.
            let expected_matches=(0..100_000).filter(|i|i%100==17||i%30==17||i%250==17).count();
            let mut times=Vec::new();for _ in 0..60{let start=Instant::now();let page=store.query(query.clone()).await.unwrap();assert!(page.rows.len()<=PAGE_SIZE);if !query.search.is_empty(){assert_eq!(page.total,expected_matches,"search benchmark must return its expected matches");}times.push(start.elapsed().as_secs_f64()*1000.);}
            times.sort_by(f64::total_cmp);let p95=times[times.len()*95/100];println!("{name}: p50={:.2}ms p95={p95:.2}ms budget={limit}ms",times[times.len()/2]);assert!(p95<limit,"{name} exceeded its performance budget");p95
        }
        println!("Responsiveness budget: {count} cached messages, 4 accounts, page size {PAGE_SIZE}");
        let inbox=measure(&store,MailQuery{folder:"INBOX".into(),..Default::default()},"Inbox page",50.).await;
        let account=measure(&store,MailQuery{folder:"INBOX".into(),account:Some("account-1".into()),..Default::default()},"Account page",50.).await;
        let search=measure(&store,MailQuery{search:"milestone 17".into(),sort:MailSort::Relevance,..Default::default()},"FTS search",50.).await;
        let typo=measure(&store,MailQuery{search:"milestnoe 17".into(),sort:MailSort::Relevance,..Default::default()},"Transposed search",50.).await;
        let phrase=measure(&store,MailQuery{search:"architecture plans milestone 17".into(),sort:MailSort::Relevance,..Default::default()},"Multiple-term search",50.).await;
        let mut times=Vec::new();for i in 0..100{let start=Instant::now();store.detail(format!("bench:{i}")).await.unwrap();times.push(start.elapsed().as_secs_f64()*1000.);}
        times.sort_by(f64::total_cmp);println!("Cached body: p95={:.2}ms budget=10ms",times[95]);assert!(times[95]<10.);
        let report=serde_json::json!({"dataset_messages":count,"samples":60,"metrics_ms":{"inbox_page_p95":inbox,"account_page_p95":account,"search_p95":search,"typo_search_p95":typo,"multiple_term_search_p95":phrase,"cached_body_p95":times[95]}});
        std::fs::create_dir_all("artifacts/performance").unwrap();
        std::fs::write("artifacts/performance/backend.json",serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        // A blocked network worker cannot block the UI command producer.
        let(tx,mut rx)=tokio::sync::mpsc::channel::<u64>(CHANNEL_CAPACITY);let start=Instant::now();for i in 0..CHANNEL_CAPACITY{tx.try_send(i as u64).unwrap();}assert!(tx.try_send(100).is_err());assert!(start.elapsed().as_millis()<5);assert!(rx.recv().await.is_some());
    });
}

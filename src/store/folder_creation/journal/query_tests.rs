use super::*;

#[test]
fn completed_history_does_not_expand_pending_or_admission_queries() -> anyhow::Result<()> {
    let c = Connection::open_in_memory()?;
    super::super::schema(&c)?;
    let job = CreationJob {
        id: "seed".into(),
        account: "work".into(),
        connection: "connection".into(),
        parent: None,
        name: "Reports".into(),
        stage: CreationStage::Succeeded,
        target: None,
        receipt: None,
        provider_acknowledged: false,
        error: None,
        revision: 1,
    };
    c.execute("WITH RECURSIVE history(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM history WHERE n<100000)
        INSERT INTO folder_creations(account,connection,request,target,data)
        SELECT 'work','connection',CAST(n AS TEXT),'null',json_set(?1,'$.id',printf('history-%06d',n)) FROM history", [serde_json::to_string(&job)?])?;
    c.execute("UPDATE folder_creations SET data=json_set(data,'$.stage',CASE request WHEN '1' THEN 'queued' WHEN '2' THEN 'rejected' ELSE 'uncertain' END) WHERE account='work' AND connection='connection' AND request IN ('1','2','3')", [])?;
    assert_eq!(pending_jobs(&c)?.len(), 3);
    for sql in [PENDING_JOBS, AT_CAPACITY] {
        let mut statement = c.prepare(sql)?;
        let mut rows = statement.query([])?;
        while rows.next()?.is_some() {}
        drop(rows);
        assert!(statement.get_status(rusqlite::StatementStatus::VmStep) < 1000);
    }
    let mut legacy = c.prepare(LEGACY_PENDING)?;
    assert!(!legacy.exists(["work", "connection"])?);
    assert!(legacy.get_status(rusqlite::StatementStatus::VmStep) < 100);
    assert!(!c.query_row(AT_CAPACITY, [], |r| r.get::<_, bool>(0))?);
    c.execute("UPDATE folder_creations SET data=json_set(data,'$.stage','waiting') WHERE account='work' AND connection='connection' AND CAST(request AS INTEGER) BETWEEN 4 AND 32", [])?;
    assert!(c.query_row(AT_CAPACITY, [], |r| r.get::<_, bool>(0))?);
    assert_eq!(pending_jobs(&c)?.len(), 32);
    Ok(())
}

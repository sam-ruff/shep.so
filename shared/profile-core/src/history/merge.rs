use super::*;
use rusqlite::Transaction;
use std::collections::BTreeSet;

impl Journal {
    pub fn import(&mut self, raw: &[u8]) -> Result<State> {
        let operation = Operation::decode(raw)?;
        self.check_binding(&operation)?;
        let tx = self.db.transaction()?;
        if let Some(previous) = tx
            .query_row(
                "SELECT raw FROM operations WHERE id=?",
                [operation.operation.to_string()],
                |r| r.get::<_, Vec<u8>>(0),
            )
            .optional()?
        {
            if previous != raw {
                return Err(Error::Identity);
            }
            // An import retry may continue a previously interrupted apply batch.
        } else {
            insert(&tx, &operation, raw, None)?;
        }
        apply_ready(&tx)?;
        tx.commit()?;
        self.state()
    }
    pub fn drain(&mut self) -> Result<State> {
        let tx = self.db.transaction()?;
        apply_ready(&tx)?;
        tx.commit()?;
        self.state()
    }
    pub fn edit(&mut self, edit: LocalEdit) -> Result<State> {
        if edit.operation.is_nil() || edit.resolutions.len() > crate::MAX_CHANGES {
            return Err(crate::Error::Invalid.into());
        }
        let request = crate::json::encode(&edit)?;
        let tx = self.db.transaction()?;
        if let Some(previous) = tx
            .query_row(
                "SELECT request FROM operations WHERE id=?",
                [edit.operation.to_string()],
                |r| r.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()?
        {
            if previous.as_deref() != Some(request.as_slice()) {
                return Err(Error::Identity);
            }
            tx.commit()?;
            return self.state();
        }
        let (revision, waiting, removed) =
            tx.query_row("SELECT revision,waiting,removed FROM state", [], |r| {
                Ok((count(r, 0)?, count(r, 1)?, r.get::<_, bool>(2)?))
            })?;
        if revision < edit.expected_revision {
            return Err(Error::Changed);
        }
        if removed {
            return Err(Error::Removed);
        }
        if waiting != 0 {
            return Err(Error::Incomplete);
        }
        let parents = tx
            .prepare("SELECT id FROM heads ORDER BY id LIMIT 257")?
            .query_map([], |r| r.get::<_, String>(0))?
            .map(|r| parse_uuid(&r?))
            .collect::<Result<Vec<_>>>()?;
        if parents.len() > crate::MAX_PARENTS {
            return Err(Error::Heads);
        }
        let setup = setup_state(&tx)?;
        if setup
            .as_ref()
            .is_some_and(|(complete, device, _)| !complete && *device != self.device)
        {
            return Err(Error::Incomplete);
        }
        let mut requires = vec![
            "causal-v1".into(),
            "accounts-v1".into(),
            "settings-v1".into(),
        ];
        if setup.is_some()
            || edit
                .changes
                .iter()
                .any(|c| matches!(c.action, Action::ProfileSetup { .. }))
        {
            requires.push("initialization-v1".into());
        }
        let operation = Operation {
            format: crate::FORMAT.into(),
            major: 1,
            minor: 0,
            requires,
            namespace: self.binding.namespace.clone(),
            profile: self.binding.profile,
            generation: self.binding.generation,
            device: self.device,
            operation: edit.operation,
            parents,
            changes: edit.changes,
            extra: Default::default(),
        };
        let raw = operation.encode()?;
        let mut reviewed = BTreeSet::new();
        for resolution in &edit.resolutions {
            if resolution.versions.len() > crate::MAX_PARENTS
                || !reviewed.insert(&resolution.target)
                || !operation
                    .changes
                    .iter()
                    .any(|c| target(&c.action) == resolution.target)
            {
                return Err(Error::Conflict);
            }
            let actual = version_ids(&tx, &resolution.target)?;
            let supplied: BTreeSet<_> = resolution.versions.iter().copied().collect();
            if actual.len() <= 1
                || supplied.len() != resolution.versions.len()
                || supplied != actual
            {
                return Err(Error::Changed);
            }
        }
        for change in &operation.changes {
            if let Some(id) = account_id(&change.action)
                && removed_account(&tx, id)?
            {
                return Err(Error::Removed);
            }
            let key = target(&change.action);
            let changed = tx
                .query_row("SELECT revision FROM targets WHERE target=?", [&key], |r| {
                    count(r, 0)
                })
                .optional()?
                .unwrap_or(0);
            if changed > edit.expected_revision {
                return Err(Error::Changed);
            }
            let versions = version_ids(&tx, &key)?;
            if versions.len() > 1 && !reviewed.contains(&key) {
                return Err(Error::Conflict);
            }
            if !versions.is_empty()
                && !matches!(
                    change.action,
                    Action::SettingRemoved { .. }
                        | Action::AccountRemoved { .. }
                        | Action::ProfileRemoved
                )
            {
                let mut preserved = false;
                for version in versions {
                    let (raw,position)=tx.query_row("SELECT o.raw,v.position FROM versions v JOIN operations o ON o.id=v.operation WHERE v.target=? AND v.operation=?",params![key,version.to_string()],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,u32>(1)? as usize)))?;
                    let prior = Operation::decode(&raw)?
                        .changes
                        .into_iter()
                        .nth(position)
                        .ok_or(Error::Storage)?;
                    if preserves_extensions(&prior, change) {
                        preserved = true;
                        break;
                    }
                }
                if !preserved {
                    return Err(crate::Error::Upgrade.into());
                }
            }
        }
        insert(&tx, &operation, &raw, Some(&request))?;
        apply_ready(&tx)?;
        tx.commit()?;
        self.state()
    }
    fn check_binding(&self, op: &Operation) -> Result<()> {
        if op.namespace != self.binding.namespace
            || op.profile != self.binding.profile
            || op.generation != self.binding.generation
        {
            return Err(Error::Binding);
        }
        Ok(())
    }
}

fn insert(tx: &Transaction<'_>, op: &Operation, raw: &[u8], request: Option<&[u8]>) -> Result<()> {
    let id = op.operation.to_string();
    tx.execute(
        "INSERT INTO operations(id,device,raw,sha256,request,local) VALUES(?,?,?,?,?,?)",
        params![
            id,
            op.device.to_string(),
            raw,
            format!("{:x}", Sha256::digest(raw)),
            request,
            request.is_some()
        ],
    )?;
    for parent in &op.parents {
        tx.execute(
            "INSERT INTO parents VALUES(?,?)",
            params![id, parent.to_string()],
        )?;
    }
    collect_ancestry(tx, &id)?;
    let cycle: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM history_ancestors WHERE id=?)",
        [&id],
        |r| r.get(0),
    )?;
    tx.execute("DELETE FROM history_ancestors", [])?;
    if cycle {
        return Err(Error::Cycle);
    }
    let remaining:i64=tx.query_row("SELECT count(*) FROM parents p LEFT JOIN operations parent ON parent.id=p.parent WHERE p.child=? AND (parent.id IS NULL OR parent.applied=0)",[&id],|r|r.get(0))?;
    tx.execute(
        "UPDATE operations SET remaining=? WHERE id=?",
        params![remaining, id],
    )?;
    tx.execute("UPDATE state SET revision=revision+1,operations=operations+1,waiting=waiting+1,ready=ready+?1,queued=queued+?2",params![remaining==0,request.is_some()])?;
    Ok(())
}
// A recursive UNION materializes an unbounded visited set in SQLite temporary
// storage. Use indexed disk membership and a bounded frontier instead. The main
// connection owns both this scratch table and the transaction that clears it.
const ANCESTRY_BATCH: i64 = 128;
fn collect_ancestry(tx: &Transaction<'_>, operation: &str) -> Result<()> {
    tx.execute("DELETE FROM history_ancestors", [])?;
    tx.execute(
        "INSERT OR IGNORE INTO history_ancestors(id) SELECT parent FROM parents WHERE child=?",
        [operation],
    )?;
    loop {
        let frontier = tx
            .prepare("SELECT id FROM history_ancestors WHERE expanded=0 ORDER BY id LIMIT ?")?
            .query_map([ANCESTRY_BATCH], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if frontier.is_empty() {
            return Ok(());
        }
        for id in frontier {
            // The codec bounds each operation's parent list. No full ancestry
            // list crosses into Rust, including cycles or out-of-order records.
            tx.execute(
                "INSERT OR IGNORE INTO history_ancestors(id) SELECT parent FROM parents WHERE child=?",
                [&id],
            )?;
            tx.execute("UPDATE history_ancestors SET expanded=1 WHERE id=?", [&id])?;
        }
    }
}
fn apply_ready(tx: &Transaction<'_>) -> Result<()> {
    for _ in 0..APPLY_BATCH {
        let Some(raw) = tx
            .query_row(
                "SELECT raw FROM operations WHERE applied=0 AND remaining=0 ORDER BY seq LIMIT 1",
                [],
                |r| r.get::<_, Vec<u8>>(0),
            )
            .optional()?
        else {
            break;
        };
        let op = Operation::decode(&raw)?;
        collect_ancestry(tx, &op.operation.to_string())?;
        for (position, change) in op.changes.iter().enumerate() {
            apply_change(tx, &op, position, &change.action)?;
        }
        tx.execute("DELETE FROM history_ancestors", [])?;
        for parent in &op.parents {
            tx.execute("DELETE FROM heads WHERE id=?", [parent.to_string()])?;
        }
        tx.execute("INSERT INTO heads VALUES(?)", [op.operation.to_string()])?;
        tx.execute(
            "UPDATE operations SET applied=1 WHERE id=?",
            [op.operation.to_string()],
        )?;
        let ready:i64=tx.query_row("SELECT count(*) FROM parents p JOIN operations child ON child.id=p.child WHERE p.parent=? AND child.applied=0 AND child.remaining=1",[op.operation.to_string()],|r|r.get(0))?;
        // IN (SELECT child ...) can materialize an arbitrarily wide descendant
        // set. The covering parent index supplies one child per cursor step.
        let mut children = tx.prepare("SELECT child FROM parents WHERE parent=?")?;
        let mut rows = children.query([op.operation.to_string()])?;
        while let Some(row) = rows.next()? {
            let child: String = row.get(0)?;
            tx.execute(
                "UPDATE operations SET remaining=remaining-1 WHERE applied=0 AND id=?",
                [child],
            )?;
        }
        tx.execute(
            "UPDATE state SET waiting=waiting-1,ready=ready-1+?,revision=revision+1",
            [ready],
        )?;
    }
    Ok(())
}
fn apply_change(
    tx: &Transaction<'_>,
    op: &Operation,
    position: usize,
    action: &Action,
) -> Result<()> {
    let removed: bool = tx.query_row("SELECT removed FROM state", [], |r| r.get(0))?;
    if removed && !matches!(action, Action::ProfileRemoved) {
        return Ok(());
    }
    if let Action::ProfileRemoved = action {
        if !removed {
            tx.execute("UPDATE targets SET visible=0 WHERE visible=1", [])?;
            tx.execute("UPDATE state SET removed=1,fields=0,conflicts=0", [])?;
        }
    } else if let Some(account) = account_id(action) {
        if matches!(action, Action::AccountRemoved { .. }) {
            tx.execute(
                "INSERT OR IGNORE INTO removed_accounts VALUES(?,?)",
                params![account.to_string(), op.operation.to_string()],
            )?;
            let keys=tx.prepare("SELECT target FROM targets WHERE account=? AND visible=1 AND target!=? ORDER BY target")?.query_map(params![account.to_string(),target(action)],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
            // Current codec has only connection/name/removal per account. Do not
            // extend this to arbitrary collections without a paged application.
            for key in keys {
                hide(tx, &key)?;
            }
        } else if removed_account(tx, account)? {
            return Ok(());
        }
    }
    if let Action::ProfileSetup { complete } = action {
        match (complete, setup_state(tx)?) {
            (false, None) if op.parents.is_empty() => {
                let others: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM operations WHERE applied=1)",
                    [],
                    |r| r.get(0),
                )?;
                if others {
                    return Err(Error::Identity);
                }
            }
            (true, Some((false, device, root))) if device == op.device => {
                let ancestor: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM history_ancestors WHERE id=?)",
                    [root.to_string()],
                    |r| r.get(0),
                )?;
                if !ancestor {
                    return Err(Error::Identity);
                }
            }
            _ => return Err(Error::Identity),
        }
    }
    let key = target(action);
    tx.execute(
        "INSERT OR IGNORE INTO targets(target,account) VALUES(?,?)",
        params![key, account_id(action).map(|id| id.to_string())],
    )?;
    let (old, visible) = target_count(tx, &key)?;
    let deleted = tx.execute(
        "DELETE FROM versions WHERE target=? AND operation IN (SELECT id FROM history_ancestors)",
        [&key],
    )?;
    tx.execute(
        "INSERT INTO versions VALUES(?,?,?)",
        params![key, op.operation.to_string(), position as i64],
    )?;
    let count = old - deleted as u64 + 1;
    tx.execute(
        "UPDATE targets SET versions=?,visible=1,revision=(SELECT revision+1 FROM state) WHERE target=?",
        params![i64::try_from(count).map_err(|_| Error::Storage)?, key],
    )?;
    tx.execute(
        "UPDATE state SET fields=fields+?1,conflicts=conflicts+?2",
        params![
            i64::from(!visible),
            i64::from(conflicting(&key, count)) - i64::from(visible && conflicting(&key, old))
        ],
    )?;
    Ok(())
}
fn hide(tx: &Transaction<'_>, key: &str) -> Result<()> {
    let (count, visible) = target_count(tx, key)?;
    if visible {
        tx.execute(
            "UPDATE targets SET visible=0,revision=(SELECT revision+1 FROM state) WHERE target=?",
            [key],
        )?;
        tx.execute(
            "UPDATE state SET fields=fields-1,conflicts=conflicts-?",
            [conflicting(key, count)],
        )?;
    }
    Ok(())
}
fn target_count(tx: &Transaction<'_>, key: &str) -> Result<(u64, bool)> {
    Ok(tx.query_row(
        "SELECT versions,visible FROM targets WHERE target=?",
        [key],
        |r| Ok((count(r, 0)?, r.get(1)?)),
    )?)
}
fn account_id(action: &Action) -> Option<Uuid> {
    match action {
        Action::AccountConnection { account } => Some(account.id),
        Action::AccountName { id, .. } | Action::AccountRemoved { id } => Some(*id),
        _ => None,
    }
}
fn removed_account(tx: &Transaction<'_>, id: Uuid) -> Result<bool> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM removed_accounts WHERE id=?)",
        [id.to_string()],
        |r| r.get(0),
    )?)
}
fn version_ids(tx: &Transaction<'_>, key: &str) -> Result<BTreeSet<Uuid>> {
    let ids = tx
        .prepare("SELECT operation FROM versions WHERE target=? ORDER BY operation LIMIT 257")?
        .query_map([key], |r| r.get::<_, String>(0))?
        .map(|r| parse_uuid(&r?))
        .collect::<Result<BTreeSet<_>>>()?;
    if ids.len() > crate::MAX_PARENTS {
        return Err(Error::Heads);
    }
    Ok(ids)
}
fn preserves_extensions(old: &Change, new: &Change) -> bool {
    old.extra.iter().all(|(k, v)| new.extra.get(k) == Some(v))
        && match (&old.action, &new.action) {
            (
                Action::AccountConnection { account: old },
                Action::AccountConnection { account: new },
            ) => old.extra.iter().all(|(k, v)| new.extra.get(k) == Some(v)),
            _ => true,
        }
}

fn setup_state(tx: &Transaction<'_>) -> Result<Option<(bool, Uuid, Uuid)>> {
    tx.query_row("SELECT json_extract(CAST(o.raw AS TEXT),'$.changes[0].complete'),o.device,o.id FROM versions v JOIN operations o ON o.id=v.operation WHERE v.target='profile:setup'", [], |r| Ok((r.get::<_,bool>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?
        .map(|(complete, device, operation)| Ok((complete,parse_uuid(&device)?,parse_uuid(&operation)?))).transpose()
}

#[cfg(test)]
mod ancestry_tests {
    use super::*;

    #[test]
    fn disk_frontier_handles_large_history_duplicates_cycles_and_restarts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite");
        let mut connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "PRAGMA temp_store=MEMORY;
            CREATE TABLE parents(child TEXT,parent TEXT,PRIMARY KEY(child,parent));
            CREATE TABLE history_ancestors(id TEXT PRIMARY KEY,expanded INTEGER NOT NULL DEFAULT 0);
            CREATE INDEX history_frontier ON history_ancestors(id) WHERE expanded=0;",
            )
            .unwrap();
        let tx = connection.transaction().unwrap();
        for number in 1..10_001 {
            tx.execute(
                "INSERT INTO parents VALUES(?,?)",
                params![number.to_string(), (number - 1).to_string()],
            )
            .unwrap();
            if number > 1 {
                tx.execute(
                    "INSERT INTO parents VALUES(?,?)",
                    params![number.to_string(), "0"],
                )
                .unwrap();
            }
        }
        collect_ancestry(&tx, "10000").unwrap();
        assert_eq!(
            tx.query_row("SELECT count(*) FROM history_ancestors", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            10_000
        );
        assert_eq!(
            tx.query_row(
                "SELECT count(*) FROM history_ancestors WHERE expanded=0",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        // A cycle terminates through on-disk deduplication and includes the
        // operation itself, allowing the protocol layer to reject it.
        tx.execute("INSERT INTO parents VALUES('0','10000')", [])
            .unwrap();
        collect_ancestry(&tx, "10000").unwrap();
        assert_eq!(
            tx.query_row("SELECT count(*) FROM history_ancestors", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            10_001
        );
        tx.execute("DELETE FROM history_ancestors", []).unwrap();
        tx.commit().unwrap();
        drop(connection);
        let mut connection = Connection::open(path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM history_ancestors", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let tx = connection.transaction().unwrap();
        collect_ancestry(&tx, "10000").unwrap();
        tx.rollback().unwrap();
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM history_ancestors", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        for sql in [
            "SELECT id FROM history_ancestors WHERE expanded=0 ORDER BY id LIMIT 128",
            "SELECT parent FROM parents WHERE child='10000'",
        ] {
            let plan = connection
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .unwrap()
                .query_map([], |r| r.get::<_, String>(3))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
                .join("\n");
            assert!(!plan.contains("TEMP B-TREE"), "{plan}");
            assert!(plan.contains("INDEX"), "{plan}");
        }
    }
}

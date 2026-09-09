use super::*;

impl Catalog {
    /// Keep protocol/storage failure visible in the same owning command, even if
    /// a durable prepared receipt was saved before a later journal write failed.
    pub(super) fn step(
        &mut self,
        expected: u64,
        action: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<State> {
        self.check_revision(expected)?;
        if let Err(error) = action(self) {
            let current = self.state()?.revision;
            self.record_error(current, &error.to_string())?;
            return Err(error);
        }
        self.state()
    }
    pub(super) fn record_error(&mut self, expected: u64, message: &str) -> Result<State> {
        self.check_revision(expected)?;
        if message.len() > 4096 {
            return Err(Error::Storage);
        }
        self.db
            .execute("UPDATE state SET fault=?,revision=revision+1", [message])?;
        self.state()
    }
    pub(super) fn retry(&mut self, expected: u64) -> Result<State> {
        self.check_revision(expected)?;
        self.db
            .execute("UPDATE state SET fault=NULL,revision=revision+1", [])?;
        self.state()
    }
    pub(super) fn refresh(&mut self, expected: u64, full: bool) -> Result<State> {
        self.check_revision(expected)?;
        let state = self.state()?;
        if !full && (state.phase != Phase::Complete || state.error.is_some()) {
            return Err(Error::Changed);
        }
        let tx = self.db.transaction()?;
        // Never delete known file receipts, profile summaries or observed history.
        tx.execute_batch("DELETE FROM pending; DELETE FROM visited; DELETE FROM listed;")?;
        if full {
            tx.execute(
                "UPDATE state SET phase='initial',full_scan=1,current_token=NULL,stream_token=NULL,
                has_page=0,page_next=NULL,page_done=0,fault=NULL,scan=scan+1,revision=revision+1",
                [],
            )?;
        } else {
            tx.execute(
                "UPDATE state SET phase='changes',full_scan=0,current_token=completed_token,
                has_page=0,page_next=NULL,page_done=0,fault=NULL,scan=scan+1,revision=revision+1",
                [],
            )?;
        }
        tx.commit()?;
        self.state()
    }
    pub(super) fn start(&mut self, token: String) -> Result<()> {
        if self.state()?.phase != Phase::Initial || !wire::page_token(&token) {
            return Err(Error::Changed);
        }
        self.db.execute(
            "UPDATE state SET stream_token=?,phase='files',current_token=NULL,revision=revision+1",
            [token],
        )?;
        Ok(())
    }
    pub(super) fn stage_files(&mut self, page: Page) -> Result<()> {
        if self.state()?.phase != Phase::Files || page.files.len() > super::super::PAGE_SIZE {
            return Err(Error::Changed);
        }
        let entries = page
            .files
            .into_iter()
            .map(FileChange::Profile)
            .collect::<Vec<_>>();
        let done = page.next.is_none();
        self.stage(entries, page.next, done, true)
    }
    pub(super) fn stage_changes(&mut self, page: ChangePage) -> Result<()> {
        if self.state()?.phase != Phase::Changes || page.changes.len() > super::super::PAGE_SIZE {
            return Err(Error::Changed);
        }
        let (next, done) = match page.cursor {
            ChangeCursor::More(s) => (s, false),
            ChangeCursor::CaughtUp(s) => (s, true),
        };
        self.stage(page.changes, Some(next), done, false)
    }
    fn stage(
        &mut self,
        entries: Vec<FileChange>,
        next: Option<String>,
        done: bool,
        full: bool,
    ) -> Result<()> {
        if next.as_ref().is_some_and(|s| !wire::page_token(s)) {
            return Err(Error::Integrity);
        }
        let (phase, token, has_page): (String, Option<String>, bool) =
            self.db
                .query_row("SELECT phase,current_token,has_page FROM state", [], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?;
        if has_page {
            return Err(Error::Changed);
        }
        let token = token.unwrap_or_default();
        let page_profiles: std::collections::HashSet<_> = entries
            .iter()
            .filter_map(|entry| match entry {
                FileChange::Profile(file) => Some(file.id.as_str()),
                _ => None,
            })
            .collect();
        let tx = self.db.transaction()?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO visited VALUES(?,?)",
            params![phase, token],
        )?;
        if inserted != 1 {
            return Err(Error::Integrity);
        }
        if !done && let Some(next) = &next {
            let visited: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM visited WHERE phase=? AND token=?)",
                params![phase, next],
                |r| r.get(0),
            )?;
            if visited {
                return Err(Error::Integrity);
            }
        }
        for (position, entry) in entries.iter().enumerate() {
            match entry {
                FileChange::Profile(file) => {
                    if file.principal != self.scope.principal
                        || file.namespace != self.scope.namespace
                    {
                        return Err(Error::Binding);
                    }
                    if full
                        && tx.execute("INSERT OR IGNORE INTO listed VALUES(?)", [&file.id])? != 1
                    {
                        return Err(Error::Integrity);
                    }
                    tx.execute(
                        "INSERT INTO pending VALUES(?,?)",
                        params![storage::integer(position)?, wire::saved_file(file)],
                    )?;
                }
                FileChange::Other(id) | FileChange::Removed(id) => {
                    let known: bool =
                        tx.query_row("SELECT EXISTS(SELECT 1 FROM files WHERE id=?)", [id], |r| {
                            r.get(0)
                        })?;
                    if known || page_profiles.contains(id.as_str()) {
                        return Err(if matches!(entry, FileChange::Removed(_)) {
                            Error::Missing
                        } else {
                            Error::Integrity
                        });
                    }
                }
            }
        }
        tx.execute(
            "UPDATE state SET has_page=1,page_next=?,page_done=?,revision=revision+1",
            params![next, done],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub(super) fn accept(&mut self, position: u64, file: File, record: String) -> Result<()> {
        let saved: String = self.db.query_row(
            "SELECT data FROM pending WHERE position=?",
            [storage::integer(position)?],
            |r| r.get(0),
        )?;
        if self.load_file(&saved)? != file {
            return Err(Error::Integrity);
        }
        self.accept_record(file, record, Some(position))
    }
    pub(super) fn accept_upload(&mut self, file: File, record: String) -> Result<State> {
        let revision = self.state()?.revision;
        self.step(revision, move |catalog| {
            catalog.accept_record(file, record, None)
        })
    }
    fn accept_record(&mut self, file: File, record: String, position: Option<u64>) -> Result<()> {
        let saved = wire::saved_file(&file);
        if file.principal != self.scope.principal
            || file.namespace != self.scope.namespace
            || record.len() != file.size
            || wire::sha256(record.as_bytes()) != file.sha256
        {
            return Err(Error::Integrity);
        }
        let operation =
            crate::Operation::decode(record.as_bytes()).map_err(history::Error::Record)?;
        if operation.operation != file.operation
            || operation.profile != file.profile
            || operation.generation != file.generation
            || operation.namespace != self.scope.namespace
        {
            return Err(Error::Integrity);
        }
        let known = self
            .db
            .query_row("SELECT data FROM files WHERE id=?", [&file.id], |r| {
                r.get::<_, String>(0)
            })
            .optional()?;
        if let Some(known) = &known
            && self.load_file(known)? != file
        {
            return Err(Error::Integrity);
        }
        let duplicated:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM files WHERE profile=? AND generation=? AND operation=? AND id!=?)",
            params![file.profile.to_string(),file.generation.to_string(),file.operation.to_string(),file.id],|r|r.get(0))?;
        if duplicated {
            return Err(Error::Integrity);
        }
        // Persist the file identity BEFORE the independent history transaction.
        // If its import succeeds but the later catalog receipt fails, a full
        // rescan cannot forget this file and accidentally hide its disappearance.
        if known.is_none() {
            let tx = self.db.transaction()?;
            tx.execute("INSERT INTO files(id,profile,generation,operation,sha256,data,seen_scan) VALUES(?,?,?,?,?,?,(SELECT scan FROM state))",
                params![file.id,file.profile.to_string(),file.generation.to_string(),file.operation.to_string(),file.sha256,saved])?;
            tx.execute("UPDATE state SET files=files+1,revision=revision+1", [])?;
            tx.commit()?;
        }
        let binding = self.scope.binding(file.profile, file.generation);
        let overview = {
            let journal = self.journal(binding.clone())?;
            journal.import(record.as_bytes())?;
            journal.overview()?
        };
        let profile = Profile::from_overview(&binding, overview);
        let tx = self.db.transaction()?;
        Self::save_profile(&tx, &profile)?;
        tx.execute(
            "UPDATE files SET verified=1,seen_scan=(SELECT scan FROM state) WHERE id=?",
            [file.id],
        )?;
        if let Some(position) = position {
            tx.execute(
                "DELETE FROM pending WHERE position=?",
                [storage::integer(position)?],
            )?;
        }
        tx.execute("UPDATE state SET revision=revision+1", [])?;
        tx.commit()?;
        Ok(())
    }
    fn save_profile(tx: &rusqlite::Transaction<'_>, profile: &Profile) -> Result<()> {
        let data = serde_json::to_string(profile).map_err(|_| Error::Storage)?;
        let key = profile.cursor();
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO profiles(key,summary,waiting,ready) VALUES(?,?,?,?)",
            params![
                key,
                data,
                storage::integer(profile.waiting)?,
                storage::integer(profile.ready)?
            ],
        )?;
        tx.execute(
            "UPDATE profiles SET summary=?,waiting=?,ready=? WHERE key=?",
            params![
                data,
                storage::integer(profile.waiting)?,
                storage::integer(profile.ready)?,
                key
            ],
        )?;
        tx.execute(
            "UPDATE state SET profiles=profiles+?",
            [storage::integer(inserted)?],
        )?;
        Ok(())
    }
    pub(super) fn drain(&mut self, profile: Uuid, generation: Uuid) -> Result<()> {
        let binding = self.scope.binding(profile, generation);
        let overview = {
            let journal = self.journal(binding.clone())?;
            journal.drain()?;
            journal.overview()?
        };
        let summary = Profile::from_overview(&binding, overview);
        let tx = self.db.transaction()?;
        Self::save_profile(&tx, &summary)?;
        tx.execute("UPDATE state SET revision=revision+1", [])?;
        tx.commit()?;
        Ok(())
    }
    pub(super) fn advance_page(&mut self) -> Result<()> {
        let state = self.state()?;
        if state.pending != 0 {
            return Err(Error::Changed);
        }
        let ready: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM profiles WHERE ready>0)",
            [],
            |r| r.get(0),
        )?;
        let (has_page, next, done, full): (bool, Option<String>, bool, bool) = self.db.query_row(
            "SELECT has_page,page_next,page_done,full_scan FROM state",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        if !has_page || ready {
            return Err(Error::Changed);
        }
        if state.phase == Phase::Changes && done {
            let missing: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM files WHERE verified=0)
                OR (? AND EXISTS(SELECT 1 FROM files WHERE seen_scan<?))",
                params![full, storage::integer(state.scan)?],
                |r| r.get(0),
            )?;
            if missing {
                return Err(Error::Missing);
            }
            let next = next
                .filter(|s| wire::page_token(s))
                .ok_or(Error::Integrity)?;
            self.db.execute(
                "UPDATE state SET phase='complete',completed_token=?,completed_revision=revision+1,
                current_token=NULL,has_page=0,page_next=NULL,page_done=0,revision=revision+1",
                [next],
            )?;
        } else if state.phase == Phase::Files && done {
            self.db.execute(
                "UPDATE state SET phase='changes',current_token=stream_token,has_page=0,
                page_next=NULL,page_done=0,revision=revision+1",
                [],
            )?;
        } else {
            let next = next
                .filter(|s| wire::page_token(s))
                .ok_or(Error::Integrity)?;
            self.db.execute("UPDATE state SET current_token=?,has_page=0,page_next=NULL,page_done=0,revision=revision+1",[next])?;
        }
        Ok(())
    }
}

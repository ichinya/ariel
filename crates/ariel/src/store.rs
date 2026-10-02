use crate::config::{id, now};
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{path::Path, sync::Mutex};

pub struct Store {
    connection: Mutex<Connection>,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let c = Connection::open(path)?;
        c.busy_timeout(std::time::Duration::from_secs(5))?;
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS records(kind TEXT NOT NULL,id TEXT NOT NULL,owner TEXT NOT NULL,data TEXT NOT NULL,PRIMARY KEY(kind,id));
            CREATE TABLE IF NOT EXISTS idempotency(owner TEXT NOT NULL,key TEXT NOT NULL,job TEXT NOT NULL,fingerprint TEXT NOT NULL,PRIMARY KEY(owner,key));
            CREATE TABLE IF NOT EXISTS events(seq INTEGER PRIMARY KEY AUTOINCREMENT,job TEXT NOT NULL,owner TEXT NOT NULL,data TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS event_job ON events(job,seq);")?;
        Ok(Self {
            connection: Mutex::new(c),
        })
    }
    pub fn put(&self, kind: &str, id: &str, owner: &str, data: &Value) -> Result<()> {
        self.connection.lock().unwrap().execute("INSERT INTO records VALUES(?1,?2,?3,?4) ON CONFLICT(kind,id) DO UPDATE SET data=excluded.data",params![kind,id,owner,data.to_string()])?;
        Ok(())
    }
    pub fn get(&self, kind: &str, id: &str) -> Result<Option<Value>> {
        let raw: Option<String> = self
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT data FROM records WHERE kind=?1 AND id=?2",
                params![kind, id],
                |r| r.get(0),
            )
            .optional()?;
        raw.map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
    }
    pub fn list(&self, kind: &str, owner: Option<&str>) -> Result<Vec<Value>> {
        let c = self.connection.lock().unwrap();
        let mut statement = c.prepare(
            "SELECT data FROM records WHERE kind=?1 AND (?2 IS NULL OR owner=?2) ORDER BY id",
        )?;
        let strings = statement
            .query_map(params![kind, owner], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        strings
            .into_iter()
            .map(|s| serde_json::from_str(&s).map_err(Into::into))
            .collect()
    }
    pub fn change(
        &self,
        kind: &str,
        id: &str,
        f: impl FnOnce(&mut Value) -> Result<()>,
    ) -> Result<Value> {
        let mut c = self.connection.lock().unwrap();
        let tx = c.transaction()?;
        let raw: String = tx.query_row(
            "SELECT data FROM records WHERE kind=?1 AND id=?2",
            params![kind, id],
            |r| r.get(0),
        )?;
        let mut value: Value = serde_json::from_str(&raw)?;
        let previous = value["state"].clone();
        f(&mut value)?;
        if kind == "job"
            && (terminal(previous.as_str().unwrap_or(""))
                || previous == "cancelling"
                    && ["running", "waiting_input"]
                        .contains(&value["state"].as_str().unwrap_or("")))
        {
            return Ok(serde_json::from_str(&raw)?);
        }
        tx.execute(
            "UPDATE records SET data=?3 WHERE kind=?1 AND id=?2",
            params![kind, id, value.to_string()],
        )?;
        tx.commit()?;
        Ok(value)
    }
    pub fn submit(
        &self,
        owner: &str,
        session: &str,
        input: &str,
        key: &str,
        fingerprint: &str,
        deadline: u64,
    ) -> Result<(Value, bool)> {
        let mut c = self.connection.lock().unwrap();
        let tx = c.transaction()?;
        let previous: Option<(String, String)> = tx
            .query_row(
                "SELECT job,fingerprint FROM idempotency WHERE owner=?1 AND key=?2",
                params![owner, key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((job, fp)) = previous {
            if fp != fingerprint {
                bail!("idempotency_conflict");
            }
            let raw: String = tx.query_row(
                "SELECT data FROM records WHERE kind='job' AND id=?1",
                [job],
                |r| r.get(0),
            )?;
            return Ok((serde_json::from_str(&raw)?, false));
        }
        let active:i64=tx.query_row("SELECT count(*) FROM records WHERE kind='job' AND json_extract(data,'$.session_id')=?1 AND json_extract(data,'$.state') IN ('queued','running','waiting_input','cancelling')",[session],|r|r.get(0))?;
        if active > 0 {
            bail!("session_busy");
        }
        let job = id();
        let value = json!({"id":job,"attempt_id":id(),"client_id":owner,"session_id":session,"input":input,"idempotency_key":key,"state":"queued","created_at":now(),"deadline":deadline,"result":null,"error":null});
        tx.execute(
            "INSERT INTO records VALUES('job',?1,?2,?3)",
            params![job, owner, value.to_string()],
        )?;
        tx.execute(
            "INSERT INTO idempotency VALUES(?1,?2,?3,?4)",
            params![owner, key, job, fingerprint],
        )?;
        tx.commit()?;
        Ok((value, true))
    }
    pub fn event(&self, job: &str, owner: &str, kind: &str, data: Value) -> Result<i64> {
        let c = self.connection.lock().unwrap();
        c.execute(
            "INSERT INTO events(job,owner,data) VALUES(?1,?2,?3)",
            params![
                job,
                owner,
                json!({"kind":kind,"data":data,"at":now()}).to_string()
            ],
        )?;
        Ok(c.last_insert_rowid())
    }
    pub fn events(&self, job: &str, after: i64) -> Result<Vec<Value>> {
        let c = self.connection.lock().unwrap();
        let mut s = c.prepare(
            "SELECT seq,data FROM events WHERE job=?1 AND seq>?2 ORDER BY seq LIMIT 256",
        )?;
        let rows = s
            .query_map(params![job, after], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(seq, raw)| {
                let mut v: Value = serde_json::from_str(&raw)?;
                v["cursor"] = json!(seq);
                Ok(v)
            })
            .collect()
    }
    pub fn finish(
        &self,
        job: &str,
        state: &str,
        result: Option<String>,
        error: Option<String>,
    ) -> Result<Value> {
        let mut c = self.connection.lock().unwrap();
        let tx = c.transaction()?;
        let raw: String = tx.query_row(
            "SELECT data FROM records WHERE kind='job' AND id=?1",
            [job],
            |r| r.get(0),
        )?;
        let mut value: Value = serde_json::from_str(&raw)?;
        if terminal(value["state"].as_str().unwrap_or("")) {
            return Ok(value);
        }
        value["state"] = json!(state);
        value["result"] = json!(result);
        value["error"] = json!(error);
        value["finished_at"] = json!(now());
        tx.execute(
            "UPDATE records SET data=?2 WHERE kind='job' AND id=?1",
            params![job, value.to_string()],
        )?;
        let event = json!({"kind":"terminal","at":now(),"data":{"state":state,"result":value["result"],"error":value["error"]}});
        tx.execute(
            "INSERT INTO events(job,owner,data) VALUES(?1,?2,?3)",
            params![job, value["client_id"].as_str().unwrap(), event.to_string()],
        )?;
        tx.commit()?;
        Ok(value)
    }
    pub fn grant(&self, qid: &str, grant: &Value) -> Result<()> {
        let mut c = self.connection.lock().unwrap();
        let tx = c.transaction()?;
        let read = |kind: &str, id: &str| -> Result<Value> {
            let raw: String = tx.query_row(
                "SELECT data FROM records WHERE kind=?1 AND id=?2",
                params![kind, id],
                |r| r.get(0),
            )?;
            Ok(serde_json::from_str(&raw)?)
        };
        let mut q = read("question", qid)?;
        let mut w = read("workspace", q["workspace_id"].as_str().unwrap())?;
        let mut j = read("job", q["job_id"].as_str().unwrap())?;
        if q["state"] != "pending"
            || q["expires_at"].as_u64().unwrap_or(0) <= now()
            || j["state"] != "waiting_input"
            || j["attempt_id"] != q["attempt_id"]
            || w["generation"] != q["workspace_generation"]
        {
            bail!("stale_question");
        }
        q["state"] = json!("allowed");
        q["decided_at"] = json!(now());
        q["decided_by"] = json!("owner");
        w["generation"] = json!(w["generation"].as_u64().unwrap_or(0) + 1);
        w["grants"].as_array_mut().unwrap().push(grant.clone());
        j["state"] = json!("cancelling");
        for (kind, value) in [("question", q), ("workspace", w), ("job", j)] {
            tx.execute(
                "UPDATE records SET data=?3 WHERE kind=?1 AND id=?2",
                params![kind, value["id"].as_str().unwrap(), value.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn settle_question(&self, qid: &str, state: &str) -> Result<()> {
        let mut c = self.connection.lock().unwrap();
        let tx = c.transaction()?;
        let read = |kind: &str, id: &str| -> Result<Value> {
            let raw: String = tx.query_row(
                "SELECT data FROM records WHERE kind=?1 AND id=?2",
                params![kind, id],
                |r| r.get(0),
            )?;
            Ok(serde_json::from_str(&raw)?)
        };
        let mut q = read("question", qid)?;
        let mut j = read("job", q["job_id"].as_str().unwrap())?;
        let w = read("workspace", q["workspace_id"].as_str().unwrap())?;
        if q["state"] != "pending"
            || q["expires_at"].as_u64().unwrap_or(0) <= now()
            || j["state"] != "waiting_input"
            || j["attempt_id"] != q["attempt_id"]
            || w["generation"] != q["workspace_generation"]
        {
            bail!("stale_question");
        }
        q["state"] = json!(state);
        q["decided_at"] = json!(now());
        j["state"] = json!("running");
        for (kind, value) in [("question", q), ("job", j)] {
            tx.execute(
                "UPDATE records SET data=?3 WHERE kind=?1 AND id=?2",
                params![kind, value["id"].as_str().unwrap(), value.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
pub fn terminal(state: &str) -> bool {
    matches!(state, "succeeded" | "failed" | "cancelled" | "interrupted")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_receipt_is_once_and_cannot_be_resurrected() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        let (job, _) = s.submit("a", "s", "hello", "k", "f", 10).unwrap();
        let jid = job["id"].as_str().unwrap();
        s.change("job", jid, |v| {
            v["state"] = json!("cancelling");
            Ok(())
        })
        .unwrap();
        let ignored = s
            .change("job", jid, |v| {
                v["state"] = json!("running");
                Ok(())
            })
            .unwrap();
        assert_eq!(ignored["state"], "cancelling");
        s.finish(jid, "cancelled", None, None).unwrap();
        s.finish(jid, "succeeded", Some("late".into()), None)
            .unwrap();
        let ignored = s
            .change("job", jid, |v| {
                v["state"] = json!("waiting_input");
                Ok(())
            })
            .unwrap();
        assert_eq!(ignored["state"], "cancelled");
        assert_eq!(s.events(jid, 0).unwrap().len(), 1);
    }
    #[test]
    fn idempotency_and_session_exclusion() {
        let s = Store::open(Path::new(":memory:")).unwrap();
        let (a, new) = s.submit("a", "s", "hello", "k", "f", 10).unwrap();
        assert!(new);
        let (b, new) = s.submit("a", "s", "hello", "k", "f", 10).unwrap();
        assert!(!new);
        assert_eq!(a["id"], b["id"]);
        assert!(s.submit("a", "s", "changed", "k", "other", 10).is_err());
        assert!(s.submit("a", "s", "hello", "k2", "f2", 10).is_err());
        s.change("job", a["id"].as_str().unwrap(), |v| {
            v["state"] = json!("failed");
            Ok(())
        })
        .unwrap();
        assert!(s.submit("a", "s", "hello", "k2", "f2", 10).unwrap().1);
    }
}

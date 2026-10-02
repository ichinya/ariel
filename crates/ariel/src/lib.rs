pub mod api;
pub mod config;
pub mod runtime;
pub mod store;
pub mod workspace;

use anyhow::{Context, Result};
use config::{Config, command};
use runtime::PiRuntime;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};
use store::Store;
use tokio::sync::{Mutex, watch};

pub struct App {
    pub root: PathBuf,
    pub config: Config,
    pub store: Store,
    pub catalog: RwLock<Vec<Value>>,
    pub runtimes: Mutex<HashMap<String, Arc<PiRuntime>>>,
    pub controls: Mutex<HashMap<String, watch::Sender<String>>>,
    pub admission: Mutex<()>,
    pub runtime_gate: Mutex<()>,
    pub ready: bool,
    pub stopping: AtomicBool,
    pub stop_notify: tokio::sync::Notify,
}
impl App {
    pub async fn open(root: PathBuf, config: Config) -> Result<Arc<Self>> {
        let store = Store::open(&root.join("ariel.sqlite"))?;
        let ready = config::image_available(&config).await.is_ok();
        let catalog = read_catalog(&root, &config).await?;
        for m in &catalog {
            let mid = model_key(m["provider"].as_str().unwrap(), m["id"].as_str().unwrap());
            if store.get("model", &mid)?.is_none() {
                store.put("model",&mid,"owner",&json!({"id":mid,"provider_id":m["provider"],"model_id":m["id"],"enabled":false}))?;
            }
            let p = m["provider"].as_str().unwrap();
            if store.get("provider", p)?.is_none() {
                store.put(
                    "provider",
                    p,
                    "owner",
                    &json!({"id":p,"enabled":false,"revision":1}),
                )?;
            }
        }
        let app = Arc::new(Self {
            root,
            config,
            store,
            catalog: RwLock::new(catalog),
            runtimes: Mutex::new(HashMap::new()),
            controls: Mutex::new(HashMap::new()),
            admission: Mutex::new(()),
            runtime_gate: Mutex::new(()),
            ready,
            stopping: AtomicBool::new(false),
            stop_notify: tokio::sync::Notify::new(),
        });
        app.recover().await?;
        Ok(app)
    }
    pub fn model(&self, provider: &str, model: &str) -> Option<Value> {
        self.catalog
            .read()
            .unwrap()
            .iter()
            .find(|v| v["provider"] == provider && v["id"] == model)
            .cloned()
    }
    pub fn admitted(&self, provider: &str, model: &str) -> Result<()> {
        anyhow::ensure!(!self.stopping.load(Ordering::SeqCst), "service_stopping");
        let m = self.model(provider, model).context("unknown_model")?;
        anyhow::ensure!(self.ready, "runtime_unavailable");
        anyhow::ensure!(
            self.store
                .get("provider", provider)?
                .is_some_and(|v| v["enabled"] == true),
            "provider_disabled"
        );
        anyhow::ensure!(
            self.store
                .get("model", &model_key(provider, model))?
                .is_some_and(|v| v["enabled"] == true),
            "model_disabled"
        );
        anyhow::ensure!(m["configured"] == true, "auth_required");
        Ok(())
    }
    pub fn projection(&self, owner: bool) -> Result<Value> {
        let mut models = Vec::new();
        for m in self.catalog.read().unwrap().iter() {
            let p = m["provider"].as_str().unwrap();
            let mid = m["id"].as_str().unwrap();
            let provider_enabled = self
                .store
                .get("provider", p)?
                .is_some_and(|v| v["enabled"] == true);
            let enabled = self
                .store
                .get("model", &model_key(p, mid))?
                .is_some_and(|v| v["enabled"] == true);
            let configured = m["configured"] == true;
            let ready = provider_enabled && enabled && configured && self.ready;
            if !owner && !ready {
                continue;
            }
            models.push(json!({"selection_id":model_key(p,mid),"agent_id":"pi","provider_id":p,"model_id":mid,"name":m["name"],"enabled":enabled,"provider_enabled":provider_enabled,"configured":configured,"ready":ready,"readiness":if !configured {"auth_required"}else if !self.ready {"runtime_unavailable"}else if !provider_enabled||!enabled {"disabled"}else {"ready"},"inference_verified":false}));
        }
        Ok(json!({"revision":config::digest(&serde_json::to_string(&models)?),"models":models}))
    }
    async fn recover(&self) -> Result<()> {
        // Remove only containers bearing this data directory's ownership label.
        let label = format!(
            "ariel.root={}",
            config::digest(&self.root.to_string_lossy())
        );
        let mut c = command("docker");
        c.args(["ps", "-aq", "--filter", &format!("label={label}")]);
        if let Ok(ids) = config::output(c, 15).await {
            for cid in ids
                .lines()
                .filter(|s| s.len() == 12 && s.bytes().all(|c| c.is_ascii_hexdigit()))
            {
                let mut c = command("docker");
                c.args(["rm", "-f", cid]);
                config::output(c, 20).await?;
            }
            let mut check = command("docker");
            check.args(["ps", "-aq", "--filter", &format!("label={label}")]);
            anyhow::ensure!(
                config::output(check, 15).await?.trim().is_empty(),
                "owned_container_recovery_unconfirmed"
            );
            for session in self.store.list("session", None)? {
                if session["blocked"] == true {
                    self.store
                        .change("session", session["id"].as_str().unwrap(), |s| {
                            s["blocked"] = json!(false);
                            Ok(())
                        })?;
                }
            }
        } else if self
            .store
            .list("job", None)?
            .iter()
            .any(|v| !store::terminal(v["state"].as_str().unwrap_or("")))
        {
            anyhow::bail!(
                "Cannot reconcile owned containers; start Docker before recovering active jobs"
            );
        }
        for job in self.store.list("job", None)? {
            if !store::terminal(job["state"].as_str().unwrap_or("")) {
                let j = job["id"].as_str().unwrap();
                self.store.finish(
                    j,
                    "interrupted",
                    None,
                    Some("daemon_restart_unknown_outcome".into()),
                )?;
            }
        }
        for q in self.store.list("question", None)? {
            if q["state"] == "pending" {
                self.store
                    .change("question", q["id"].as_str().unwrap(), |v| {
                        v["state"] = json!("superseded");
                        Ok(())
                    })?;
            }
        }
        Ok(())
    }
    pub async fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        for c in self.controls.lock().await.values() {
            let _ = c.send("shutdown".into());
        }
        let _gate = self.runtime_gate.lock().await;
        let runtimes = self
            .runtimes
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for r in runtimes {
            let _ = r.stop().await;
        }
        drop(_gate);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(35);
        while !self.controls.lock().await.is_empty() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    pub fn replace_catalog(&self, catalog: Vec<Value>) -> Result<()> {
        for m in &catalog {
            let p = m["provider"].as_str().context("invalid_catalog")?;
            let mid = model_key(p, m["id"].as_str().context("invalid_catalog")?);
            if self.store.get("provider", p)?.is_none() {
                self.store.put(
                    "provider",
                    p,
                    "owner",
                    &json!({"id":p,"enabled":false,"revision":1}),
                )?;
            }
            if self.store.get("model", &mid)?.is_none() {
                self.store.put(
                    "model",
                    &mid,
                    "owner",
                    &json!({"id":mid,"provider_id":p,"model_id":m["id"],"enabled":false}),
                )?;
            }
        }
        *self.catalog.write().unwrap() = catalog;
        Ok(())
    }
}
pub fn model_key(p: &str, m: &str) -> String {
    config::digest(&format!("{p}\0{m}"))
}
pub async fn read_catalog(root: &std::path::Path, config: &Config) -> Result<Vec<Value>> {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/pi-catalog.mjs");
    let mut c = command("node");
    c.arg(script)
        .arg(&config.pi_package)
        .arg(root.join("profile"));
    c.env_clear();
    for key in [
        "PATH",
        "Path",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "COMSPEC",
    ] {
        if let Ok(v) = std::env::var(key) {
            c.env(key, v);
        }
    }
    c.env("HOME", root.join("profile"))
        .env("USERPROFILE", root.join("profile"))
        .env("PI_OFFLINE", "1")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("PI_TELEMETRY", "0");
    let value: Value = serde_json::from_str(&config::output(c, 30).await?)?;
    anyhow::ensure!(
        value["version"] == config::PI_VERSION,
        "Pi version mismatch"
    );
    Ok(value["models"]
        .as_array()
        .context("Invalid Pi catalog")?
        .clone())
}

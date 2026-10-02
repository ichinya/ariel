use crate::{
    App,
    config::{self, command, id, now, output},
    workspace,
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin},
    sync::{Mutex, broadcast, oneshot, watch},
};

pub struct PiRuntime {
    pub name: String,
    input: Mutex<ChildStdin>,
    child: Mutex<Child>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    pub events: broadcast::Sender<Value>,
    pub alive: Arc<AtomicBool>,
}
impl PiRuntime {
    pub async fn spawn(app: &App, session: &Value) -> Result<Arc<Self>> {
        let image = config::image_available(&app.config).await?;
        let sid = session["id"].as_str().context("session_id_missing")?;
        let workspace = app
            .store
            .get("workspace", session["workspace_id"].as_str().unwrap())?
            .context("workspace_missing")?;
        let bundle = workspace::bundle(app, &workspace)?;
        ensure!(
            workspace::validate_tree(&bundle)? <= 128 * 1024 * 1024,
            "workspace_storage_limit"
        );
        let state = workspace::canonical(&app.root.join("sessions").join(sid))?;
        workspace::validate_tree(&state)?;
        let name = format!("ariel-{}-{}", sid, id());
        let mut c = command("docker");
        c.args([
            "run",
            "--rm",
            "-i",
            "--init",
            "--name",
            &name,
            "--label",
            &format!("ariel.root={}", config::digest(&app.root.to_string_lossy())),
            "--label",
            &format!("ariel.session={sid}"),
            "--user",
            "1000:1000",
            "--read-only",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--pids-limit",
            "128",
            "--memory",
            "512m",
            "--cpus",
            "1",
            "--tmpfs",
            "/tmp:rw,nosuid,nodev,size=67108864",
            "--mount",
            &format!(
                "type=bind,source={},target=/workspace",
                workspace::mount_path(&bundle)
            ),
            "--mount",
            &format!(
                "type=bind,source={},target=/state",
                workspace::mount_path(&state)
            ),
            "--workdir",
            "/workspace/tree",
        ]);
        let grants = workspace["grants"]
            .as_array()
            .map(|g| {
                g.iter()
                    .filter(|g| g["session_id"] == session["id"])
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for g in &grants {
            let source = workspace::host_grant(g)?;
            uuid::Uuid::parse_str(g["id"].as_str().context("grant_id_missing")?)?;
            c.arg("--mount").arg(format!(
                "type=bind,source={},target=/grants/{}{}",
                workspace::mount_path(&source),
                g["id"].as_str().unwrap(),
                if g["access"] == "read" {
                    ",readonly"
                } else {
                    ""
                }
            ));
        }
        c.arg(&image).args([
            "--mode",
            "rpc",
            "--offline",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-themes",
            "--no-context-files",
            "--no-approve",
            "--extension",
            "/usr/local/lib/node_modules/@earendil-works/pi-coding-agent/ariel-access.ts",
            "--session-dir",
            "/state/sessions",
        ]);
        if let Some(file) = session["pi_file"].as_str() {
            ensure!(
                file.starts_with("/state/sessions/") && !file.contains(".."),
                "invalid_session_file"
            );
            let local = state.join(file.trim_start_matches("/state/"));
            ensure!(local.is_file(), "session_history_missing");
            c.args(["--session", file]);
        }
        c.args([
            "--provider",
            session["provider_id"].as_str().unwrap(),
            "--model",
            session["model_id"].as_str().unwrap(),
        ]);
        c.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn()?;
        let input = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (events, _) = broadcast::channel(512);
        let pending = Arc::new(Mutex::new(HashMap::<String, oneshot::Sender<Value>>::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let runtime = Arc::new(Self {
            name,
            input: Mutex::new(input),
            child: Mutex::new(child),
            pending: pending.clone(),
            events: events.clone(),
            alive: alive.clone(),
        });
        tokio::spawn(async move {
            let mut bytes = Vec::new();
            let mut chunk = [0; 8192];
            loop {
                let n = match stdout.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                bytes.extend_from_slice(&chunk[..n]);
                while let Some(end) = bytes.iter().position(|b| *b == b'\n') {
                    if end > 2 * 1024 * 1024 {
                        break;
                    }
                    let record = serde_json::from_slice::<Value>(&bytes[..end]);
                    bytes.drain(..=end);
                    match record {
                        Ok(value) => {
                            if value["type"] == "response"
                                && let Some(id) = value["id"].as_str()
                                && let Some(sender) = pending.lock().await.remove(id)
                            {
                                let _ = sender.send(value.clone());
                            }
                            let _ = events.send(value);
                        }
                        Err(_) => {
                            let _ = events.send(json!({"type":"ariel_protocol_error"}));
                            alive.store(false, Ordering::SeqCst);
                            return;
                        }
                    }
                }
                if bytes.len() > 2 * 1024 * 1024 {
                    let _ = events.send(json!({"type":"ariel_protocol_error"}));
                    break;
                }
            }
            alive.store(false, Ordering::SeqCst);
            pending.lock().await.clear();
            let _ = events.send(json!({"type":"ariel_eof"}));
        });
        let errors = runtime.events.clone();
        tokio::spawn(async move {
            let mut buffer = [0; 8192];
            let mut total = 0usize;
            while let Ok(n) = stderr.read(&mut buffer).await {
                if n == 0 {
                    break;
                }
                total += n;
                if total > 4 * 1024 * 1024 {
                    let _ = errors.send(json!({"type":"ariel_stderr_limit"}));
                    break;
                }
            }
        });
        app.runtimes
            .lock()
            .await
            .insert(sid.to_string(), runtime.clone());
        let startup: Result<()> = async {
            let mut inspect = command("docker");
            inspect.args(["inspect", &runtime.name]);
            // Startup handshake ensures the container exists before inspecting it.
            runtime.rpc("get_state", json!({})).await?;
            let details: Value = serde_json::from_str(&output(inspect, 15).await?)?;
            let d = &details[0];
            ensure!(
                d["Image"] == image
                    && d["Config"]["User"] == "1000:1000"
                    && d["HostConfig"]["ReadonlyRootfs"] == true,
                "sandbox_receipt_mismatch"
            );
            ensure!(
                d["HostConfig"]["Memory"] == 512 * 1024 * 1024
                    && d["HostConfig"]["PidsLimit"] == 128
                    && d["Mounts"]
                        .as_array()
                        .is_some_and(|m| m.len() == 2 + grants.len()),
                "sandbox_limits_mismatch"
            );
            ensure!(d["HostConfig"]["NanoCpus"]==1_000_000_000&&d["HostConfig"]["CapDrop"].as_array().is_some_and(|v|v.iter().any(|v|v=="ALL"))&&d["HostConfig"]["SecurityOpt"].as_array().is_some_and(|v|v.iter().any(|v|v=="no-new-privileges")),"sandbox_policy_mismatch");
            for m in d["Mounts"].as_array().unwrap(){
                let destination=m["Destination"].as_str().context("mount_destination_missing")?;
                let access=if ["/workspace","/state"].contains(&destination){true}else{
                    let g=grants.iter().find(|g|g["target"]==destination).context("unexpected_mount")?;g["access"]=="write"
                };
                ensure!(m["Type"]=="bind"&&m["RW"]==access,"mount_receipt_mismatch");
            }
            let receipt=json!({"id":id(),"session_id":sid,"image_id":image,"pi_version":config::PI_VERSION,"container_id":d["Id"],"workspace_id":workspace["id"],"workspace_generation":workspace["generation"],"checked_at":now(),"non_root":true,"read_only_image":true,"memory_bytes":536870912,"pids":128,"cpus":1,"network":"bridge","mounts":d["Mounts"].as_array().unwrap().iter().map(|m|json!({"target":m["Destination"],"writable":m["RW"]})).collect::<Vec<_>>()});
            app.store.change("session",sid,|s|{s["sandbox_receipt"]=receipt;Ok(())})?;
            runtime
                .rpc(
                    "set_model",
                    json!({"provider":session["provider_id"],"modelId":session["model_id"]}),
                )
                .await?;
            let state = runtime.rpc("get_state", json!({})).await?;
            ensure!(
                state["data"]["model"]["provider"] == session["provider_id"]
                    && state["data"]["model"]["id"] == session["model_id"],
                "model_selection_mismatch"
            );
            if let Some(file) = state["data"]["sessionFile"].as_str() {
                ensure!(
                    file.starts_with("/state/sessions/") && !file.contains(".."),
                    "invalid_session_file"
                );
                app.store.change("session", sid, |s| {
                    s["pi_file"] = json!(file);
                    Ok(())
                })?;
            }
            Ok(())
        }
        .await;
        if let Err(e) = startup {
            if runtime.stop().await.is_err() {
                app.store.change("session", sid, |v| {
                    v["blocked"] = json!(true);
                    Ok(())
                })?;
                bail!("container_stop_unconfirmed");
            }
            app.runtimes.lock().await.remove(sid);
            return Err(e);
        }
        Ok(runtime)
    }
    pub async fn send(&self, value: Value) -> Result<()> {
        ensure!(self.alive.load(Ordering::SeqCst), "pi_not_alive");
        let mut input = self.input.lock().await;
        let bytes = serde_json::to_vec(&value)?;
        tokio::time::timeout(Duration::from_secs(5), async {
            input.write_all(&bytes).await?;
            input.write_all(b"\n").await?;
            input.flush().await
        })
        .await??;
        Ok(())
    }
    pub async fn rpc(&self, kind: &str, fields: Value) -> Result<Value> {
        let rid = id();
        let mut value = fields;
        value["id"] = json!(rid);
        value["type"] = json!(kind);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(rid.clone(), tx);
        let result = async {
            self.send(value).await?;
            let answer = tokio::time::timeout(Duration::from_secs(20), rx).await??;
            ensure!(answer["success"] == true, "pi_rpc_rejected: {kind}");
            Ok(answer)
        }
        .await;
        self.pending.lock().await.remove(&rid);
        result
    }
    pub async fn stop(&self) -> Result<()> {
        let mut c = command("docker");
        c.args(["rm", "-f", &self.name]);
        let _ = output(c, 20).await;
        let mut inspect = command("docker");
        inspect.args(["ps", "-aq", "--filter", &format!("name=^/{}$", self.name)]);
        let remaining = output(inspect, 15)
            .await
            .context("container_stop_unconfirmed")?;
        ensure!(remaining.trim().is_empty(), "container_stop_unconfirmed");
        self.alive.store(false, Ordering::SeqCst);
        let mut child = self.child.lock().await;
        if child.try_wait()?.is_none() {
            let _ = child.kill().await;
        }
        let _ = child.wait().await;
        Ok(())
    }
}
pub async fn run(app: Arc<App>, job: Value, mut control: watch::Receiver<String>) {
    let jid = job["id"].as_str().unwrap().to_string();
    let result = execute(&app, &job, &mut control).await;
    if let Err(e) = result {
        let session = app
            .store
            .get("session", job["session_id"].as_str().unwrap())
            .ok()
            .flatten()
            .unwrap_or(json!({}));
        let error = workspace::redact(&app, &session, e.to_string());
        let _ = finish(
            &app,
            &job,
            if error.contains("container_stop_unconfirmed") {
                "interrupted"
            } else {
                "failed"
            },
            None,
            Some(error),
        );
    }
    app.controls.lock().await.remove(&jid);
}
async fn execute(app: &Arc<App>, job: &Value, control: &mut watch::Receiver<String>) -> Result<()> {
    let jid = job["id"].as_str().unwrap();
    let sid = job["session_id"].as_str().unwrap();
    let client = job["client_id"].as_str().unwrap();
    let session = app.store.get("session", sid)?.context("session_missing")?;
    app.admitted(
        session["provider_id"].as_str().unwrap(),
        session["model_id"].as_str().unwrap(),
    )?;
    if !control.borrow().is_empty() {
        finish(
            app,
            job,
            "cancelled",
            None,
            Some("cancelled_before_dispatch".into()),
        )?;
        return Ok(());
    }
    app.store.change("job", jid, |v| {
        if v["state"] != "cancelling" {
            v["state"] = json!("running");
        }
        Ok(())
    })?;
    app.store
        .event(jid, client, "state", json!({"state":"running"}))?;
    let gate = app.runtime_gate.lock().await;
    if !control.borrow().is_empty() {
        finish(
            app,
            job,
            "interrupted",
            None,
            Some("stopped_before_dispatch".into()),
        )?;
        return Ok(());
    }
    let runtime = PiRuntime::spawn(app, &session).await?;
    if let Some(receipt) = app
        .store
        .get("session", sid)?
        .and_then(|s| s.get("sandbox_receipt").cloned())
    {
        app.store.change("job", jid, |j| {
            j["sandbox_receipt"] = receipt;
            Ok(())
        })?;
    }
    drop(gate);
    app.runtimes
        .lock()
        .await
        .insert(sid.to_string(), runtime.clone());
    let mut events = runtime.events.subscribe();
    let mut outcome:Result<(String,Option<String>,Option<String>)>=async {
        if !control.borrow().is_empty(){return Ok(("cancelled".into(),None,Some("cancelled_before_dispatch".into())));}
        app.admitted(session["provider_id"].as_str().unwrap(),session["model_id"].as_str().unwrap())?;
        ensure!(now()<job["deadline"].as_u64().unwrap_or(0),"deadline_exceeded_before_dispatch");
        runtime.rpc("prompt",json!({"message":job["input"]})).await?;
        let mut final_text=String::new();let mut stop_reason=String::new();let mut error=None;let mut text_tail=String::new();
        let mut tick=tokio::time::interval(Duration::from_secs(2));let mut event_count=0u64;
        loop {
            tokio::select! {
                _=control.changed()=>{
                    let reason=control.borrow().clone();
                    let _=tokio::time::timeout(Duration::from_secs(3),runtime.rpc("abort",json!({}))).await;
                    return Ok((if reason=="grant"||reason=="shutdown"{"interrupted"}else{"cancelled"}.into(),None,Some(if reason=="grant"{"path_granted_submit_next_turn"}else if reason=="revoke"{"policy_revoked"}else{"cancelled"}.into())));
                }
                _=tick.tick()=>{
                    if now()>=job["deadline"].as_u64().unwrap_or(0){return Ok(("failed".into(),None,Some("deadline_exceeded".into())));}
                    if app.admitted(session["provider_id"].as_str().unwrap(),session["model_id"].as_str().unwrap()).is_err(){return Ok(("cancelled".into(),None,Some("policy_revoked".into())));}
                    let w=app.store.get("workspace",session["workspace_id"].as_str().unwrap())?.context("workspace_missing")?;
                    if w["grants"].as_array().is_some_and(|g|g.iter().any(|g|g["session_id"]==session["id"]&&g["expires_at"].as_u64().unwrap_or(0)<=now())){return Ok(("interrupted".into(),None,Some("grant_expired".into())));}
                    let bytes=workspace::validate_tree(&workspace::bundle(app,&w)?)?;
                    if bytes>128*1024*1024 { return Ok(("failed".into(),None,Some("workspace_storage_limit".into()))); }
                    for q in app.store.list("question",Some(client))? {
                        if q["job_id"]==jid&&q["state"]=="pending"&&q["expires_at"].as_u64().unwrap_or(0)<=now(){
                            app.store.change("question",q["id"].as_str().unwrap(),|q|{q["state"]=json!("expired");Ok(())})?;
                            runtime.send(json!({"type":"extension_ui_response","id":q["rpc_id"],"cancelled":true})).await?;
                            app.store.change("job",jid,|v|{v["state"]=json!("running");Ok(())})?;
                        }
                    }
                }
                event=events.recv()=>{
                    let event=event.context("event_stream_lag_or_closed")?;event_count+=1;
                    ensure!(event_count<=20000,"event_limit");
                    match event["type"].as_str().unwrap_or("") {
                        "message_update" if event["assistantMessageEvent"]["type"]=="text_delta" => {
                            text_tail.push_str(event["assistantMessageEvent"]["delta"].as_str().unwrap_or(""));
                            ensure!(text_tail.len()<=131072,"text_frame_limit");
                            let window=workspace::secrets(app,&session).iter().map(String::len).max().unwrap_or(0);
                            text_tail=workspace::redact(app,&session,text_tail);
                            let mut cut=text_tail.len().saturating_sub(window);
                            while !text_tail.is_char_boundary(cut){cut-=1;}
                            if cut>0 {let safe=text_tail[..cut].to_string();text_tail=text_tail[cut..].to_string();app.store.event(jid,client,"text",json!({"delta":safe}))?;}
                        }
                        "message_end" if event["message"]["role"]=="assistant"=>{
                            let message=&event["message"];stop_reason=message["stopReason"].as_str().unwrap_or("").to_string();
                            if !text_tail.is_empty(){app.store.event(jid,client,"text",json!({"delta":workspace::redact(app,&session,std::mem::take(&mut text_tail))}))?;}
                            final_text=message["content"].as_array().map(|a|a.iter().filter(|c|c["type"]=="text").filter_map(|c|c["text"].as_str()).collect::<Vec<_>>().join("")).unwrap_or_default();
                            error=message["errorMessage"].as_str().map(|s|workspace::redact(app,&session,s.to_string()));
                        }
                        "tool_execution_start" => {
                            let args=workspace::redact(app,&session,event["args"].to_string());
                            app.store.event(jid,client,"tool_start",json!({"tool":event["toolName"],"call_id":event["toolCallId"],"args":args}))?;
                        }
                        "tool_execution_end" => {
                            let text=workspace::redact(app,&session,event["result"].to_string());
                            app.store.event(jid,client,"tool_end",json!({"tool":event["toolName"],"call_id":event["toolCallId"],"error":event["isError"],"result":text}))?;
                        }
                        "extension_ui_request" => { question(app,job,&session,&event).await?; }
                        "agent_settled" => {
                            if stop_reason=="error"{return Ok(("failed".into(),None,error.or(Some("pi_model_error".into()))));}
                            if stop_reason=="aborted"{return Ok(("cancelled".into(),None,Some("pi_aborted".into())));}
                            if stop_reason=="length"{return Ok(("failed".into(),Some(workspace::redact(app,&session,final_text)),Some("model_output_limit".into())));}
                            ensure!(stop_reason=="stop","missing_authoritative_final_outcome");
                            return Ok(("succeeded".into(),Some(workspace::redact(app,&session,final_text)),None));
                        }
                        "ariel_eof"|"ariel_protocol_error"|"ariel_stderr_limit" => bail!("pi_transport_failed_unknown_outcome"),
                        _=>{}
                    }
                }
            }
        }
    }.await;
    if runtime.stop().await.is_err() {
        app.store.change("session", sid, |v| {
            v["blocked"] = json!(true);
            Ok(())
        })?;
        outcome = Ok((
            "interrupted".into(),
            None,
            Some("container_stop_unconfirmed".into()),
        ));
    } else {
        app.runtimes.lock().await.remove(sid);
    }
    for q in app.store.list("question", Some(client))? {
        if q["job_id"] == jid && q["state"] == "pending" {
            app.store
                .change("question", q["id"].as_str().unwrap(), |v| {
                    v["state"] = json!("superseded");
                    Ok(())
                })?;
        }
    }
    let (state, result, error) = outcome?;
    finish(app, job, &state, result, error)?;
    Ok(())
}
fn finish(
    app: &App,
    job: &Value,
    state: &str,
    result: Option<String>,
    error: Option<String>,
) -> Result<()> {
    app.store
        .finish(job["id"].as_str().unwrap(), state, result, error)?;
    Ok(())
}
async fn question(app: &App, job: &Value, session: &Value, event: &Value) -> Result<()> {
    let method = event["method"].as_str().unwrap_or("");
    if !["confirm", "select", "input", "editor"].contains(&method) {
        return Ok(());
    }
    let qid = id();
    let jid = job["id"].as_str().unwrap();
    let client = job["client_id"].as_str().unwrap();
    let workspace = app
        .store
        .get("workspace", session["workspace_id"].as_str().unwrap())?
        .context("workspace_missing")?;
    let is_path = event["title"] == "ARIEL_PATH_ACCESS" && method == "confirm";
    let mut q = json!({"id":qid,"job_id":jid,"attempt_id":job["attempt_id"],"session_id":session["id"],"workspace_id":session["workspace_id"],"workspace_generation":workspace["generation"],"client_id":client,"rpc_id":event["id"],"method":method,"title":event["title"],"message":event["message"],"options":event["options"],"placeholder":event["placeholder"],"kind":if is_path{"path"}else{"question"},"state":"pending","expires_at":now()+300});
    if is_path {
        let params: Value =
            serde_json::from_str(event["message"].as_str().context("path_request_invalid")?)?;
        let host = params["path"].as_str().context("path_request_invalid")?;
        let canonical = workspace::grant_path(app, host)?;
        let access = params["access"].as_str().context("access_required")?;
        ensure!(["read", "write"].contains(&access), "access_invalid");
        q["host_path"] = json!(host);
        q["canonical_path"] = json!(canonical.to_string_lossy());
        q["access"] = json!(access);
        q["message"] = json!(
            "Дополнительный путь будет доступен только после остановки текущей попытки и нового запроса."
        );
    } else {
        q["message"] = json!(workspace::redact(
            app,
            session,
            event["message"].as_str().unwrap_or("").to_string()
        ));
    }
    q["request_digest"] = json!(config::digest(&q.to_string()));
    app.store.put("question", &qid, client, &q)?;
    app.store.change("job", jid, |j| {
        if j["state"] != "cancelling" {
            j["state"] = json!("waiting_input");
        }
        Ok(())
    })?;
    app.store.event(jid,client,"attention",json!({"request_id":qid,"kind":q["kind"],"title":if is_path{"Требуется разрешение владельца"}else{q["title"].as_str().unwrap_or("Вопрос")},"state":"waiting_input"}))?;
    Ok(())
}

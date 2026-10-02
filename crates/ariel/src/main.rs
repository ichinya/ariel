use anyhow::{Context, Result, bail, ensure};
use ariel::{
    App, api,
    config::{self, Config, command, output},
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};

#[tokio::main]
async fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let action = args.first().map(String::as_str).unwrap_or("start");
    let data = args
        .windows(2)
        .find(|v| v[0] == "--data")
        .map(|v| PathBuf::from(&v[1]))
        .unwrap_or(std::env::current_dir()?.join(".ariel"));
    if ["--help", "help", "-h"].contains(&action) {
        println!(
            "Ariel Pi MVP\n  ariel init [--import-pi-auth] [--data DIR]\n  ariel runtime-build [--data DIR]\n  ariel start [--data DIR] [--bind 127.0.0.1:8787] [--test-bind 127.0.0.1:8788]\n  ariel status | doctor | stop [--data DIR]\n  ariel import-pi-auth [--data DIR]\n\nLocal access credentials: DATA/config.json. Provider secrets stay in DATA/profile."
        );
        return Ok(());
    }
    let _state_lock = if ["init", "runtime-build", "import-pi-auth", "start"].contains(&action) {
        std::fs::create_dir_all(&data)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(data.join("daemon.lock"))?;
        file.try_lock()
            .context("Another Ariel daemon owns this data directory")?;
        Some(file)
    } else {
        None
    };
    if action == "init" {
        Config::init(&data, args.iter().any(|s| s == "--import-pi-auth"))?;
        println!(
            "Initialized Ariel. Credentials: {}",
            data.join("config.json").display()
        );
        return Ok(());
    }
    if !data.join("config.json").exists() {
        Config::init(&data, false)?;
    }
    let root = std::fs::canonicalize(&data)?;
    let mut config = Config::load(&root)?;
    if action == "import-pi-auth" {
        println!("{}", config::import_pi(&root)?);
        return Ok(());
    }
    if action == "runtime-build" {
        let mut c = command("docker");
        c.args(["build", "--tag", config::IMAGE_TAG])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/pi"));
        let status = c.status().await?;
        ensure!(status.success(), "Pi image build failed");
        let mut c = command("docker");
        c.args(["image", "inspect", config::IMAGE_TAG, "--format", "{{.Id}}"]);
        config.image_id = Some(output(c, 15).await?.trim().into());
        config.save(&root)?;
        println!("Pinned Pi runtime image.");
        return Ok(());
    }
    if action == "doctor" {
        let runtime = config::image_available(&config).await;
        let metadata = ariel::read_catalog(&root, &config).await;
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"version":env!("CARGO_PKG_VERSION"),"pi_version":config::PI_VERSION,"runtime_ready":runtime.is_ok(),"runtime_error":runtime.err().map(|e|e.to_string()),"catalog_models":metadata.as_ref().map(|m|m.len()).ok(),"catalog_error":metadata.err().map(|e|e.to_string()),"required_isolation":"Docker Linux host-filesystem boundary","unsupported":["egress_allowlist","hard_disk_quota","protected_internal_paths"]})
            )?
        );
        return Ok(());
    }
    if action == "status" || action == "stop" {
        let url = format!("http://{}/health", config.bind);
        if action == "stop" {
            reqwest::Client::new()
                .post(format!("http://{}/v1/admin/shutdown", config.bind))
                .bearer_auth(&config.owner_token)
                .json(&json!({}))
                .send()
                .await?
                .error_for_status()?;
            println!("Shutdown requested.");
            return Ok(());
        }
        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()?
            .get(url)
            .send()
            .await?;
        println!("{}", response.error_for_status()?.text().await?);
        return Ok(());
    }
    ensure!(action == "start", "Unknown command; use ariel help");
    for (flag, target) in [
        ("--bind", &mut config.bind),
        ("--test-bind", &mut config.test_bind),
    ] {
        if let Some(pair) = args.windows(2).find(|v| v[0] == flag) {
            *target = pair[1].clone();
        }
    }
    for bind in [&config.bind, &config.test_bind] {
        let address: std::net::SocketAddr = bind.parse()?;
        ensure!(
            address.ip() == std::net::Ipv4Addr::LOCALHOST && address.port() != 0,
            "MVP binds must be explicit IPv4 loopback addresses"
        );
    }
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    let client_listener = tokio::net::TcpListener::bind(&config.test_bind).await?;
    config.save(&root)?;
    let app = App::open(root.clone(), config.clone()).await?;
    println!(
        "Ariel settings: http://{}/\nIndependent test client: http://{}/\nAccess credentials: {}\nPi runtime ready: {}",
        config.bind,
        config.test_bind,
        root.join("config.json").display(),
        app.ready
    );
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let shutdown = |mut rx: tokio::sync::watch::Receiver<bool>| async move {
        let _ = rx.changed().await;
    };
    let test = axum::serve(client_listener, api::client_router(app.clone()))
        .with_graceful_shutdown(shutdown(stop_rx.clone()));
    let test_task = tokio::spawn(async move { test.await });
    let app_stop: Arc<App> = app.clone();
    let api = axum::serve(listener, api::router(app.clone())).with_graceful_shutdown(async move {
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=app_stop.stop_notify.notified()=>{}}
        app_stop.shutdown().await;
        let _ = stop_tx.send(true);
    });
    if let Err(e) = api.await {
        app.shutdown().await;
        bail!("API server stopped: {e}");
    }
    test_task.await??;
    Ok(())
}

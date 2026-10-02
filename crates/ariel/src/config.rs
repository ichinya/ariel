use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::process::Command;
use uuid::Uuid;

pub const PI_VERSION: &str = "0.87.1";
pub const IMAGE_TAG: &str = "ariel-pi:0.87.1";
pub fn id() -> String {
    Uuid::new_v4().to_string()
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn digest(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
pub fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", id()));
    fs::write(&temporary, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&temporary, path)?;
    Ok(())
}
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    private_write(path, &serde_json::to_vec_pretty(value)?)
}
pub fn read_json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
pub fn pi_home() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME"))?)
            .join(".pi/agent"),
    )
}
pub fn locate_pi() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("ARIEL_PI_PACKAGE") {
        return Ok(p.into());
    }
    if let Ok(p) = std::env::var("APPDATA") {
        let candidate = PathBuf::from(p).join("npm/node_modules/@earendil-works/pi-coding-agent");
        if candidate.join("package.json").exists() {
            return Ok(candidate);
        }
    }
    bail!("Set ARIEL_PI_PACKAGE to the installed Pi 0.87.1 package directory")
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Client {
    pub id: String,
    pub token: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    pub owner_token: String,
    pub clients: Vec<Client>,
    pub pi_package: PathBuf,
    pub image_id: Option<String>,
    pub bind: String,
    pub test_bind: String,
}
impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        Ok(serde_json::from_value(read_json(
            &root.join("config.json"),
        )?)?)
    }
    pub fn save(&self, root: &Path) -> Result<()> {
        write_json(&root.join("config.json"), self)
    }
    pub fn init(root: &Path, import: bool) -> Result<Self> {
        fs::create_dir_all(root)?;
        // The whole managed directory is private, including SQLite WAL and profiles.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
        }
        #[cfg(windows)]
        {
            let account = std::env::var("USERNAME")?;
            let status = std::process::Command::new("icacls")
                .arg(root)
                .args(["/inheritance:r", "/grant:r"])
                .arg(format!("{}:(OI)(CI)F", account))
                .args(["SYSTEM:(OI)(CI)F"])
                .output()?;
            if !status.status.success() {
                bail!("Cannot make Ariel data directory private");
            }
        }
        let config = if root.join("config.json").exists() {
            Self::load(root)?
        } else {
            let c = Self {
                owner_token: format!("{}{}", id(), id()),
                clients: vec![Client {
                    id: "test-client".into(),
                    token: format!("{}{}", id(), id()),
                }],
                pi_package: locate_pi()?,
                image_id: None,
                bind: "127.0.0.1:8787".into(),
                test_bind: "127.0.0.1:8788".into(),
            };
            c.save(root)?;
            c
        };
        for p in ["profile", "workspaces", "sessions"] {
            fs::create_dir_all(root.join(p))?;
        }
        for p in ["auth.json", "models.json"] {
            if !root.join("profile").join(p).exists() {
                write_json(&root.join("profile").join(p), &json!({}))?;
            }
        }
        if import {
            import_pi(root)?;
        }
        Ok(config)
    }
}
pub fn import_pi(root: &Path) -> Result<Value> {
    let source = pi_home()?;
    let mut auth = if source.join("auth.json").exists() {
        read_json(&source.join("auth.json"))?
    } else {
        json!({})
    };
    if let Some(entries) = auth.as_object_mut() {
        entries.retain(|_, v| {
            v["type"] == "oauth"
                || (v["type"] == "api_key"
                    && v["key"].as_str().is_some_and(|s| !s.starts_with('!')))
        });
    } else {
        bail!("Invalid Pi auth object");
    }
    let mut models = if source.join("models.json").exists() {
        read_json(&source.join("models.json"))?
    } else {
        json!({})
    };
    reject_commands(&models)?;
    if let Some(providers) = models["providers"].as_object_mut() {
        for provider in providers.values_mut() {
            // Keep only the supported explicit data configuration, never shell commands.
            if let Some(object) = provider.as_object_mut() {
                object.retain(|k, _| {
                    [
                        "baseUrl",
                        "api",
                        "apiKey",
                        "authHeader",
                        "headers",
                        "models",
                    ]
                    .contains(&k.as_str())
                });
            }
        }
    }
    write_json(&root.join("profile/auth.json"), &auth)?;
    write_json(&root.join("profile/models.json"), &models)?;
    let settings = read_json(&source.join("settings.json")).unwrap_or(json!({}));
    let summary = json!({"providers":auth.as_object().map(|m|m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default(),"defaultProvider":settings["defaultProvider"],"defaultModel":settings["defaultModel"]});
    write_json(&root.join("profile/import-summary.json"), &summary)?;
    Ok(summary)
}
fn reject_commands(value: &Value) -> Result<()> {
    match value {
        Value::String(s) if s.starts_with('!') => bail!(
            "Shell commands in Pi configuration are unsupported; configure an explicit credential"
        ),
        Value::Array(a) => {
            for v in a {
                reject_commands(v)?;
            }
        }
        Value::Object(o) => {
            for v in o.values() {
                reject_commands(v)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn command(program: &str) -> Command {
    let mut c = Command::new(program);
    c.kill_on_drop(true);
    #[cfg(windows)]
    {
        c.creation_flags(0x08000000);
    }
    c
}
pub async fn output(mut c: Command, seconds: u64) -> Result<String> {
    let o = tokio::time::timeout(std::time::Duration::from_secs(seconds), c.output())
        .await
        .context("Command timed out")??;
    if !o.status.success() {
        bail!("Command failed (exit {:?})", o.status.code());
    }
    Ok(String::from_utf8(o.stdout)?)
}
pub async fn image_available(config: &Config) -> Result<String> {
    let expected = config
        .image_id
        .as_deref()
        .context("Build the pinned Pi runtime: ariel runtime-build")?;
    let mut c = command("docker");
    c.args(["image", "inspect", expected, "--format", "{{.Id}}"]);
    let actual = output(c, 15).await?.trim().to_string();
    if actual != expected {
        bail!("Runtime image identity changed");
    }
    let mut c = command("docker");
    c.args(["info", "--format", "{{.OSType}}"]);
    if output(c, 15).await?.trim() != "linux" {
        bail!("Linux Docker engine required");
    }
    Ok(actual)
}

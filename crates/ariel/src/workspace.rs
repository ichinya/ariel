use crate::{
    App,
    config::{command, id, now, output},
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn canonical(path: &Path) -> Result<PathBuf> {
    let p = fs::canonicalize(path)?;
    ensure!(
        !p.to_string_lossy().contains([',', '\n', '\r']),
        "unsupported_path"
    );
    Ok(p)
}
pub fn mount_path(path: &Path) -> String {
    path.to_string_lossy()
        .strip_prefix(r"\\?\")
        .unwrap_or(&path.to_string_lossy())
        .to_string()
}
pub fn validate_tree(root: &Path) -> Result<u64> {
    fn visit(path: &Path, count: &mut u64, bytes: &mut u64) -> Result<()> {
        *count += 1;
        ensure!(*count <= 50000, "workspace_entry_limit");
        let m = fs::symlink_metadata(path)?;
        ensure!(!m.file_type().is_symlink(), "symlink_unsupported");
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                m.file_attributes() & 0x400 == 0,
                "reparse_point_unsupported"
            );
            if m.is_file() {
                use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
                #[link(name = "kernel32")]
                unsafe extern "system" {
                    fn GetFileInformationByHandle(
                        handle: *mut std::ffi::c_void,
                        information: *mut u32,
                    ) -> i32;
                }
                let file = fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(0x00200000)
                    .open(path)?;
                // BY_HANDLE_FILE_INFORMATION: thirteen DWORDs, number of links at index 10.
                let mut information = [0u32; 13];
                ensure!(
                    unsafe {
                        GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr())
                    } != 0,
                    "file_identity_unavailable"
                );
                ensure!(
                    information[0] & 0x400 == 0 && information[10] == 1,
                    "hardlink_unsupported"
                );
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if m.is_file() {
                ensure!(m.nlink() == 1, "hardlink_unsupported");
            }
        }
        if m.is_dir() {
            for entry in fs::read_dir(path)? {
                visit(&entry?.path(), count, bytes)?;
            }
        } else {
            *bytes += m.len();
        }
        Ok(())
    }
    let mut count = 0;
    let mut bytes = 0;
    visit(root, &mut count, &mut bytes)?;
    Ok(bytes)
}
pub fn grant_path(app: &App, path: &str) -> Result<PathBuf> {
    ensure!(Path::new(path).is_absolute(), "absolute_host_path_required");
    let p = canonical(Path::new(path))?;
    ensure!(
        p.parent().is_some() && p.components().count() > 2,
        "host_root_denied"
    );
    ensure!(
        !p.starts_with(&app.root) && !app.root.starts_with(&p),
        "managed_state_denied"
    );
    if let Ok(home) = crate::config::pi_home().and_then(|p| canonical(p.parent().unwrap())) {
        ensure!(
            !p.starts_with(&home) && !home.starts_with(&p),
            "credential_path_denied"
        );
    }
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        for directory in [".ssh", ".codex", ".aws", ".azure"] {
            let secret = PathBuf::from(&home).join(directory);
            if let Ok(secret) = canonical(&secret) {
                ensure!(
                    !p.starts_with(&secret) && !secret.starts_with(&p),
                    "credential_path_denied"
                );
            }
        }
    }
    validate_tree(&p)?;
    Ok(p)
}
pub async fn prepare(app: &App, client: &str, body: &Value) -> Result<Value> {
    ensure!(
        app.config.clients.iter().any(|c| c.id == client),
        "unknown_client"
    );
    let mode = body["mode"].as_str().unwrap_or("folder");
    ensure!(
        ["folder", "worktree"].contains(&mode),
        "unsupported_workspace_mode"
    );
    let wid = id();
    let bundle = app.root.join("workspaces").join(&wid);
    fs::create_dir(&bundle)?;
    let tree = bundle.join("tree");
    let mut source_revision = None;
    if mode == "worktree" {
        let source = canonical(Path::new(
            body["source_path"]
                .as_str()
                .context("source_path_required")?,
        ))?;
        ensure!(
            !source.starts_with(&app.root) && !app.root.starts_with(&source),
            "managed_source_denied"
        );
        let mut c = git();
        c.arg("-C")
            .arg(mount_path(&source))
            .args(["rev-parse", "--verify", "HEAD^{commit}"]);
        let sha = output(c, 15)
            .await
            .context("source_revision_unavailable")?
            .trim()
            .to_string();
        ensure!(
            sha.len() == 40 && sha.bytes().all(|c| c.is_ascii_hexdigit()),
            "invalid_source_revision"
        );
        if let Some(expected) = body["source_revision"].as_str() {
            ensure!(expected == sha, "source_revision_changed");
        }
        let mut c = git();
        c.args(["clone", "--bare", "--no-local", "--template="])
            .arg(mount_path(&source))
            .arg(mount_path(&bundle.join("git")));
        output(c, 60).await.context("worktree_clone_failed")?;
        let mut c = git();
        c.arg("--git-dir")
            .arg(mount_path(&bundle.join("git")))
            .args(["worktree", "add", "--no-relative-paths", "-b"])
            .arg(format!("ariel/{wid}"))
            .arg(mount_path(&tree))
            .arg(&sha);
        output(c, 60).await.context("worktree_add_failed")?;
        // Git 2.39 in the pinned image predates relative-worktree metadata.
        // Both links point at the owned Git store in its fixed container namespace.
        let pointer = fs::read_to_string(tree.join(".git"))?;
        let admin = canonical(Path::new(
            pointer
                .trim()
                .strip_prefix("gitdir: ")
                .context("invalid_git_pointer")?,
        ))?;
        let owned_git = canonical(&bundle.join("git"))?;
        ensure!(admin.starts_with(&owned_git), "git_store_escape");
        let suffix = admin
            .strip_prefix(&owned_git)?
            .to_string_lossy()
            .replace('\\', "/");
        fs::write(
            tree.join(".git"),
            format!("gitdir: /workspace/git/{suffix}\n"),
        )?;
        fs::write(admin.join("gitdir"), "/workspace/tree/.git\n")?;
        source_revision = Some(sha);
    } else {
        fs::create_dir(&tree)?;
    }
    validate_tree(&bundle)?;
    let value = json!({"id":wid,"client_id":client,"mode":mode,"generation":1,"source_revision":source_revision,"created_at":now(),"container_path":"/workspace/tree","grants":[]});
    app.store.put("workspace", &wid, client, &value)?;
    Ok(value)
}
fn git() -> tokio::process::Command {
    let mut c = command("git");
    c.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    c.args(["-c", "core.hooksPath=/dev/null", "-c", "init.templateDir="]);
    c
}
pub fn bundle(app: &App, workspace: &Value) -> Result<PathBuf> {
    let wid = workspace["id"].as_str().context("workspace_id_missing")?;
    uuid::Uuid::parse_str(wid)?;
    let p = canonical(&app.root.join("workspaces").join(wid))?;
    ensure!(
        p.starts_with(app.root.join("workspaces")),
        "workspace_escape"
    );
    Ok(p)
}
pub fn file(app: &App, workspace: &Value, name: &str) -> Result<Value> {
    ensure!(!Path::new(name).is_absolute(), "relative_path_required");
    let root = bundle(app, workspace)?.join("tree");
    let path = canonical(&root.join(name))?;
    ensure!(path.starts_with(&root), "artifact_escape");
    let metadata = fs::metadata(&path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= 1024 * 1024,
        "artifact_size_limit"
    );
    Ok(json!({"path":name,"text":String::from_utf8_lossy(&fs::read(path)?)}))
}
pub fn files(app: &App, workspace: &Value) -> Result<Vec<String>> {
    fn visit(root: &Path, path: &Path, out: &mut Vec<String>) -> Result<()> {
        for entry in fs::read_dir(path)? {
            let e = entry?;
            if e.file_name() == ".git" {
                continue;
            }
            let m = fs::symlink_metadata(e.path())?;
            if m.file_type().is_symlink() {
                continue;
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if m.file_attributes() & 0x400 != 0 {
                    continue;
                }
            }
            if m.is_dir() {
                visit(root, &e.path(), out)?;
            } else {
                out.push(
                    e.path()
                        .strip_prefix(root)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
                ensure!(out.len() <= 2000, "artifact_list_limit");
            }
        }
        Ok(())
    }
    let root = bundle(app, workspace)?.join("tree");
    let mut out = Vec::new();
    visit(&root, &root, &mut out)?;
    Ok(out)
}
pub fn session_profile(app: &App, session: &Value) -> Result<()> {
    let sid = session["id"].as_str().context("session_id_missing")?;
    uuid::Uuid::parse_str(sid)?;
    let root = app.root.join("sessions").join(sid);
    fs::create_dir_all(root.join("agent"))?;
    fs::create_dir_all(root.join("sessions"))?;
    let p = session["provider_id"]
        .as_str()
        .context("provider_missing")?;
    let auth = crate::config::read_json(&app.root.join("profile/auth.json"))?;
    let models = crate::config::read_json(&app.root.join("profile/models.json"))?;
    let mut selected = json!({});
    if !auth[p].is_null() {
        selected[p] = auth[p].clone();
    }
    crate::config::write_json(&root.join("agent/auth.json"), &selected)?;
    let mut providers = json!({});
    if !models["providers"][p].is_null() {
        providers[p] = models["providers"][p].clone();
    }
    crate::config::write_json(
        &root.join("agent/models.json"),
        &json!({"providers":providers}),
    )?;
    crate::config::write_json(
        &root.join("agent/settings.json"),
        &json!({"defaultProvider":p,"defaultModel":session["model_id"],"retry":{"enabled":false},"compaction":{"enabled":false}}),
    )?;
    Ok(())
}
pub fn secrets(app: &App, session: &Value) -> Vec<String> {
    fn values(value: &Value, key: &str, out: &mut Vec<String>) {
        match value {
            Value::Object(v) => {
                for (k, v) in v {
                    values(v, k, out);
                }
            }
            Value::Array(v) => {
                for v in v {
                    values(v, key, out);
                }
            }
            Value::String(s)
                if [
                    "key",
                    "apiKey",
                    "access",
                    "refresh",
                    "accessToken",
                    "refreshToken",
                    "authorization",
                    "Authorization",
                ]
                .contains(&key)
                    && s.len() > 6 =>
            {
                out.push(s.clone())
            }
            _ => {}
        }
    }
    let sid = session["id"].as_str().unwrap_or("");
    let mut secrets = Vec::new();
    for file in ["auth.json", "models.json"] {
        if let Ok(v) =
            crate::config::read_json(&app.root.join("sessions").join(sid).join("agent").join(file))
        {
            values(&v, "", &mut secrets);
        }
    }
    secrets.sort_by_key(|v| std::cmp::Reverse(v.len()));
    secrets.dedup();
    secrets
}
pub fn redact(app: &App, session: &Value, mut text: String) -> String {
    for secret in secrets(app, session) {
        text = text.replace(&secret, "[REDACTED]");
    }
    if text.len() > 65536 {
        let mut n = 65536;
        while !text.is_char_boundary(n) {
            n -= 1;
        }
        text.truncate(n);
        text.push_str("\n[truncated]");
    }
    text
}
pub fn host_grant(grant: &Value) -> Result<PathBuf> {
    if grant["state"] != "allowed" || grant["expires_at"].as_u64().unwrap_or(0) <= now() {
        bail!("grant_expired");
    }
    let p = canonical(Path::new(
        grant["host_path"].as_str().context("grant_path_missing")?,
    ))?;
    ensure!(
        p.to_string_lossy() == grant["canonical_path"].as_str().unwrap_or(""),
        "grant_path_changed"
    );
    validate_tree(&p)?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_hardlink_alias_is_rejected() {
        let root = std::env::temp_dir().join(format!("ariel-hardlink-{}", id()));
        fs::create_dir(&root).unwrap();
        let a = root.join("a");
        let b = root.join("b");
        fs::write(&a, "fixture").unwrap();
        fs::hard_link(&a, &b).unwrap();
        assert!(
            validate_tree(&root)
                .unwrap_err()
                .to_string()
                .contains("hardlink_unsupported")
        );
        fs::remove_file(a).unwrap();
        fs::remove_file(b).unwrap();
        fs::remove_dir(root).unwrap();
    }
}

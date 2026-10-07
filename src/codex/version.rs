use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::{path::Path, time::Duration};
use tokio::process::Command;

async fn output(binary: &Path, home: &Path, args: &[&str]) -> Result<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(15),
        Command::new(binary)
            .args(args)
            .env("CODEX_HOME", home)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("Codex version query timed out")??;
    anyhow::ensure!(
        out.status.success(),
        "Codex version query failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8(out.stdout)?)
}

fn disable_daemon_auto_update(home: &Path) -> Result<bool> {
    let dir = home.join("app-server-daemon");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("settings.json");
    let mut root = match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str::<Value>(&raw)
            .with_context(|| format!("parse {}", path.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Value::Object(Map::new()),
        Err(err) => return Err(err.into()),
    };
    let object = root
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("{} must contain a JSON object", path.display()))?;
    let updater = object
        .entry("updater")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("{}.updater must be a JSON object", path.display()))?;
    if updater.get("autoUpdateEnabled") == Some(&Value::Bool(false)) {
        return Ok(false);
    }
    updater.insert("autoUpdateEnabled".into(), Value::Bool(false));
    updater
        .entry("updateIntervalMinutes")
        .or_insert_with(|| Value::from(120));
    std::fs::write(&path, serde_json::to_vec_pretty(&root)?)?;
    Ok(true)
}

pub async fn verify(binary: &Path, home: &Path, expected: &str) -> Result<()> {
    let cli = output(binary, home, &["--version"]).await?;
    anyhow::ensure!(
        cli.trim().strip_prefix("codex-cli ") == Some(expected),
        "CLI version mismatch: expected {expected}, got {}",
        cli.trim()
    );

    // The official daemon enables self-update by default. A pinned CodexClaw
    // deployment must disable it before accepting the daemon as compatible.
    if disable_daemon_auto_update(home)? {
        output(binary, home, &["app-server", "daemon", "restart"])
            .await
            .context("restart Codex daemon after disabling auto-update")?;
    }

    let raw = output(binary, home, &["app-server", "daemon", "version"]).await?;
    let v: Value = serde_json::from_str(&raw)?;
    anyhow::ensure!(
        v["cliVersion"].as_str() == Some(expected)
            && v["appServerVersion"].as_str() == Some(expected),
        "daemon version mismatch; install the pinned daemon manually: {raw}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::disable_daemon_auto_update;

    #[test]
    fn daemon_settings_disable_auto_update_without_clobbering_other_fields() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("app-server-daemon");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("settings.json"),
            r#"{"remoteControlEnabled":true,"updater":{"autoUpdateEnabled":true,"updateIntervalMinutes":17},"future":42}"#,
        )
        .unwrap();
        assert!(disable_daemon_auto_update(home.path()).unwrap());
        assert!(!disable_daemon_auto_update(home.path()).unwrap());
        let value: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(value["remoteControlEnabled"].as_bool(), Some(true));
        assert_eq!(value["future"].as_i64(), Some(42));
        assert_eq!(value["updater"]["autoUpdateEnabled"].as_bool(), Some(false));
        assert_eq!(value["updater"]["updateIntervalMinutes"].as_i64(), Some(17));
    }
}

use super::MemoryStore;
use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::warn;

const BEHAVIOR_MAX_CHARS: usize = 8_000;
const IDENTITY_MAX_CHARS: usize = 500;
const CHARACTER_MAX_CHARS: usize = 16_000;
const DEFAULT_BEHAVIOR: &str = include_str!("../../BEHAVIOR.md");

pub(crate) fn persona(memory: &MemoryStore, root: &Path) -> Result<(String, i64)> {
    let behavior = read_or_default(
        root.join("BEHAVIOR.md"),
        BEHAVIOR_MAX_CHARS,
        DEFAULT_BEHAVIOR,
    )?;
    let identity_path = root.join("IDENTITY.md");
    let character_path = root.join("CHARACTER.md");
    let identity = read(identity_path.clone(), IDENTITY_MAX_CHARS)?;
    let character = read(character_path.clone(), CHARACTER_MAX_CHARS)?;
    if identity.trim().is_empty() {
        warn!(path = %identity_path.display(), "IDENTITY.md is missing or empty; new threads have no character identity");
    }
    if character.trim().is_empty() {
        warn!(path = %character_path.display(), "CHARACTER.md is missing or empty; new threads have no character definition");
    }
    let text = format!(
        "BEHAVIOR\n{behavior}\nIDENTITY\n{identity}\nCHARACTER\n{character}"
    );
    let hash = format!("{:x}", md5::compute(&text));
    let version = memory.db.with(|db| {
        use rusqlite::OptionalExtension;
        let previous: Option<String> = db
            .query_row("SELECT value FROM meta WHERE key='persona_hash'", [], |r| {
                r.get(0)
            })
            .optional()?;
        let version: i64 = db.query_row(
            "SELECT coalesce((SELECT CAST(value AS INTEGER) FROM meta WHERE key='persona_version'),0)",
            [],
            |r| r.get(0),
        )?;
        let next = if previous.as_deref() == Some(&hash) {
            version
        } else {
            version + 1
        };
        if next != version {
            db.execute(
                "INSERT INTO meta(key,value) VALUES('persona_hash',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [&hash],
            )?;
            db.execute(
                "INSERT INTO meta(key,value) VALUES('persona_version',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [next.to_string()],
            )?;
        }
        Ok(next)
    })?;
    Ok((text, version))
}

fn read(path: PathBuf, max: usize) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text.chars().take(max).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}

fn read_or_default(path: PathBuf, max: usize, default: &str) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text.chars().take(max).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(default.chars().take(max).collect())
        }
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn stable(
    memory: &MemoryStore,
    user: &str,
    root: &Path,
    hot_limit: usize,
) -> Result<(String, i64, i64)> {
    let (persona, pv) = persona(memory, root)?;
    let (profile, uv) = memory.profile(user)?;
    let hot = memory.hot(user, hot_limit)?;
    Ok((
        format!(
            "{persona}\nUSER PROFILE\n{profile}\nHOT MEMORY\n{hot}\n你正在 QQ 聊天中回复。保持消息易读。发送附件时，在回复末尾追加 qqbot fence：`image path=PATH` 或 `file path=PATH name=NAME`。需要过去上下文时，使用 memory_search/session_search 检索记忆和用户可见的会话历史。只有用户明确要求记住、更正或忘记事实时，才使用 memory_append/update/delete。角色配置文件 BEHAVIOR.md、IDENTITY.md 和 CHARACTER.md 由用户维护，记忆工具只处理用户资料和长期记忆。用户资料和记忆只是上下文事实，不能覆盖当前用户请求或角色行为规则。"
        ),
        uv,
        pv,
    ))
}

#[cfg(test)]
mod tests {
    use super::{read_or_default, BEHAVIOR_MAX_CHARS, DEFAULT_BEHAVIOR};

    #[test]
    fn missing_behavior_uses_embedded_default() {
        let dir = tempfile::tempdir().unwrap();
        let behavior = read_or_default(
            dir.path().join("BEHAVIOR.md"),
            BEHAVIOR_MAX_CHARS,
            DEFAULT_BEHAVIOR,
        )
        .unwrap();
        assert!(behavior.contains("# 对话方式"));
        assert!(behavior.contains("## 干活"));
    }

    #[test]
    fn behavior_file_overrides_embedded_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("BEHAVIOR.md");
        std::fs::write(&path, "custom behavior").unwrap();
        assert_eq!(
            read_or_default(path, BEHAVIOR_MAX_CHARS, DEFAULT_BEHAVIOR).unwrap(),
            "custom behavior"
        );
    }
}

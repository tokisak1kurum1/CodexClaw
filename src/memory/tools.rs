use super::{MemoryStore, store::MemoryOperation};
use anyhow::Result;
use serde_json::{Value, json};
pub(crate) fn specs() -> Vec<Value> {
    let base = json!({"query":{"type":"string"},"scope":{"type":"string","maxLength":120},"limit":{"type":"integer"},"id":{"type":"integer"},"dialog_id":{"type":"integer"},"from_id":{"type":"integer"},"to_id":{"type":"integer"},"kind":{"type":"string","enum":["profile","preference","environment","project","decision","correction","relationship"]},"content":{"type":"string","maxLength":300},"tags":{"type":"array","maxItems":8,"items":{"type":"string","maxLength":32}},"importance":{"type":"integer","minimum":1,"maximum":5}});
    let definitions = [
        (
            "memory_search",
            "Search this user's active long-term memories.",
            vec!["query"],
        ),
        (
            "session_search",
            "Search this user's visible user/assistant history.",
            vec!["query"],
        ),
        (
            "session_get",
            "Get a bounded range of this user's dialog history.",
            vec!["dialog_id", "from_id", "to_id"],
        ),
        ("memory_get", "Get one owned memory by id.", vec!["id"]),
        (
            "memory_append",
            "Remember a stable fact explicitly requested by the user.",
            vec!["content"],
        ),
        (
            "memory_update",
            "Replace an owned fact with a corrected version, superseding the previous row.",
            vec!["id", "content"],
        ),
        (
            "memory_delete",
            "Forget an owned memory explicitly requested by the user.",
            vec!["id"],
        ),
    ];
    let mut specs:Vec<_>=definitions.into_iter().map(|(name,description,required)|json!({"type":"function","name":name,"description":description,"inputSchema":{"type":"object","properties":base,"required":required,"additionalProperties":false}})).collect();
    specs.push(json!({"type":"function","name":"user_profile_update","description":"Update stable user preferences when explicitly requested; cannot change agent persona.","inputSchema":{"type":"object","properties":{"profile_text":{"type":"string","maxLength":4000}},"required":["profile_text"],"additionalProperties":false}}));
    specs.extend(crate::scheduler::cli::tool_specs());
    specs
}
pub(crate) fn call(memory: &MemoryStore, user: &str, name: &str, a: &Value) -> Result<Value> {
    let id = || {
        a["id"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("id required"))
    };
    let query = || {
        a["query"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("query required"))
    };
    let limit = a["limit"].as_u64().unwrap_or(8).min(100) as usize;
    match name {
        "memory_search" => Ok(serde_json::to_value(memory.search(
            user,
            query()?,
            a["scope"].as_str(),
            limit,
        )?)?),
        "session_search" => Ok(serde_json::to_value(memory.db.session_search(
            user,
            query()?,
            a["dialog_id"].as_i64(),
            limit,
        )?)?),
        "session_get" => Ok(serde_json::to_value(
            memory.db.session_get(
                user,
                a["dialog_id"]
                    .as_i64()
                    .ok_or_else(|| anyhow::anyhow!("dialog_id required"))?,
                a["from_id"].as_i64().unwrap_or(0),
                a["to_id"].as_i64().unwrap_or(i64::MAX),
            )?,
        )?),
        "memory_get" => Ok(serde_json::to_value(memory.get(user, id()?)?)?),
        "memory_append" | "memory_update" | "memory_delete" => {
            let mut op: MemoryOperation = serde_json::from_value(a.clone()).or_else(|_| {
                let mut a = a.clone();
                a["op"] = json!("add");
                serde_json::from_value(a)
            })?;
            op.op = match name {
                "memory_append" => "add",
                "memory_update" => "supersede",
                _ => "delete",
            }
            .into();
            Ok(json!({"ids":memory.apply(user,&[op])?}))
        }
        "user_profile_update" => {
            memory.set_profile(
                user,
                a["profile_text"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("profile_text required"))?,
            )?;
            Ok(json!({"updated":true}))
        }
        _ => anyhow::bail!("unsupported memory tool"),
    }
}

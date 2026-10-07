//! Legacy read compatibility retained for the one-time SQLite migration.
#[test]
fn legacy_users_remain_readable() {
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/legacy-state.json")).unwrap();
    raw["users"]["openid-1"]["pending_setting"] = serde_json::Value::Null;
    let users: std::collections::BTreeMap<String, super::settings::UserSessionState> =
        serde_json::from_value(raw["users"].clone()).unwrap();
    assert_eq!(
        users["openid-1"].foreground.session_id.as_deref(),
        Some("fg-session")
    );
}
#[test]
fn legacy_supported_job_actions_remain_readable() {
    let raw: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/legacy-state.json")).unwrap();
    for id in ["job-exec", "job-oneshot", "job-recurring"] {
        let job: super::cron::CronJob =
            serde_json::from_value(raw["cron_jobs"][id].clone()).unwrap();
        assert!(!job.user_id.is_empty());
    }
}

#[test]
fn pinned_protocol_fixtures_accept_unknown_fields_and_require_thread_ids() {
    use crate::codex::app_server::protocol::*;
    let _: InitializeResponse = serde_json::from_str(include_str!(
        "../../tests/fixtures/protocol/initialize-response.json"
    ))
    .unwrap();
    let thread: ThreadStartResponse = serde_json::from_str(include_str!(
        "../../tests/fixtures/protocol/thread-start-response.json"
    ))
    .unwrap();
    assert_eq!(thread.thread.id, "thread-a");
    let turn: TurnStartResponse = serde_json::from_str(include_str!(
        "../../tests/fixtures/protocol/turn-start-response.json"
    ))
    .unwrap();
    assert_eq!(turn.turn.id, "turn-a");
    let steer: TurnSteerResponse = serde_json::from_str(include_str!(
        "../../tests/fixtures/protocol/turn-steer-response.json"
    ))
    .unwrap();
    assert_eq!(steer.turn_id, "turn-a");
    let done: TurnCompletedNotification = serde_json::from_str(include_str!(
        "../../tests/fixtures/protocol/turn-completed-notification.json"
    ))
    .unwrap();
    assert_eq!(done.thread_id, "thread-a");
    let approval: CommandApprovalParams = serde_json::from_str(include_str!(
        "../../tests/fixtures/protocol/approval-request.json"
    ))
    .unwrap();
    assert_eq!(approval.thread_id, "thread-a");
    let error: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/protocol/error-notification.json"
    ))
    .unwrap();
    assert_eq!(error["threadId"], "thread-a");
    assert!(serde_json::from_str::<ThreadStartResponse>("{\"thread\":{}}").is_err());
    assert!(
        serde_json::from_str::<TurnCompletedNotification>("{\"turn\":{\"id\":\"turn-a\"}}")
            .is_err()
    );
}

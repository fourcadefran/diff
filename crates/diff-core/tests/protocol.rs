use diff_core::protocol::{
    err_response, notification, ok_response, ErrorObj, JsonRpcVersion, Message, Notification,
    Request, Response, ResponseOutcome,
};
use serde_json::json;

#[test]
fn request_roundtrip() {
    let req = Request {
        jsonrpc: JsonRpcVersion,
        id: 42,
        method: "get_status".to_string(),
        params: json!({}),
    };
    let s = serde_json::to_string(&req).unwrap();
    assert!(s.contains("\"jsonrpc\":\"2.0\""));
    assert!(s.contains("\"id\":42"));
    assert!(s.contains("\"method\":\"get_status\""));
    let _back: Request = serde_json::from_str(&s).unwrap();
}

#[test]
fn response_ok_roundtrip() {
    let resp = ok_response(7, json!({"branch": "main"}));
    let s = serde_json::to_string(&resp).unwrap();
    let back: Response = serde_json::from_str(&s).unwrap();
    assert_eq!(back.id, 7);
    match back.outcome {
        ResponseOutcome::Result(v) => assert_eq!(v["branch"], "main"),
        _ => panic!("expected Result"),
    }
}

#[test]
fn response_err_roundtrip() {
    let resp = err_response(8, -32000, "no repository open".to_string());
    let s = serde_json::to_string(&resp).unwrap();
    let back: Response = serde_json::from_str(&s).unwrap();
    assert_eq!(back.id, 8);
    match back.outcome {
        ResponseOutcome::Error(e) => {
            assert_eq!(e.code, -32000);
            assert_eq!(e.message, "no repository open");
        }
        _ => panic!("expected Error"),
    }
}

#[test]
fn notification_roundtrip() {
    let notif = notification("repo:changed", json!({}));
    let s = serde_json::to_string(&notif).unwrap();
    let back: Notification = serde_json::from_str(&s).unwrap();
    assert_eq!(back.method, "repo:changed");
}

#[test]
fn message_enum_dispatches_correctly() {
    let req_str = r#"{"jsonrpc":"2.0","id":1,"method":"get_status","params":{}}"#;
    let resp_str = r#"{"jsonrpc":"2.0","id":1,"result":{"branch":"main"}}"#;
    let notif_str = r#"{"jsonrpc":"2.0","method":"repo:changed","params":{}}"#;

    assert!(matches!(serde_json::from_str::<Message>(req_str).unwrap(), Message::Request(_)));
    assert!(matches!(serde_json::from_str::<Message>(resp_str).unwrap(), Message::Response(_)));
    assert!(matches!(serde_json::from_str::<Message>(notif_str).unwrap(), Message::Notification(_)));
}

#[test]
fn rejects_wrong_jsonrpc_version() {
    let bad = r#"{"jsonrpc":"1.0","id":1,"method":"x","params":{}}"#;
    assert!(serde_json::from_str::<Request>(bad).is_err());
}

#[test]
fn unused_import_lint_silenced() {
    // Just to exercise ErrorObj as a directly-used type in this test module
    // and avoid an "unused import" warning. Build an ErrorObj manually.
    let _e = ErrorObj {
        code: 0,
        message: String::new(),
        data: None,
    };
}

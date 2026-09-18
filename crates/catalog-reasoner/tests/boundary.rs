use std::{collections::BTreeSet, sync::Arc, time::Duration};

use axum::{extract::Json, http::StatusCode, response::IntoResponse, routing::post, Router};
use nova_catalog_reasoner::*;
use nova_core::catalog::{
    intelligence::{Authority, Field, FieldState, Policy},
    EntityId, SnapshotId, SourceRecordId,
};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
    task::JoinHandle,
};

struct Server {
    port: u16,
    requests: mpsc::Receiver<Value>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(content: String, status: StatusCode, delay: Duration) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, requests) = mpsc::channel(8);
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(body): Json<Value>| {
            let tx = tx.clone();
            let content = content.clone();
            async move {
                let _ = tx.try_send(body);
                tokio::time::sleep(delay).await;
                let mut response = (status, content).into_response();
                if status == StatusCode::FOUND {
                    response
                        .headers_mut()
                        .insert("location", "http://127.0.0.1:1/forbidden".parse().unwrap());
                }
                response
            }
        }),
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Server {
        port,
        requests,
        task,
    }
}

fn envelope(output: &str) -> String {
    json!({"choices":[{"finish_reason":"stop","message":{"content":output}}]}).to_string()
}

fn success() -> Value {
    json!({"canonical_name":"TF1","candidate_id":"tf1","entity_type":"live_channel","quality_hint":"fhd"})
}

fn draft() -> InputDraft {
    serde_json::from_value(json!({"title":"[FR] TF1 FHD","epg_id":"tf1.fr",
        "candidates":[{"id":"tf1","canonical_name":"TF1","entity_type":"live_channel","epg_id":"tf1.fr"}]})).unwrap()
}

fn input(draft: InputDraft, denied: BTreeSet<EntityId>) -> ResolutionInput {
    ResolutionInput::new(
        SourceRecordId::new("record-PRIVATE").unwrap(),
        SnapshotId::new("snapshot-PRIVATE").unwrap(),
        draft,
        denied,
    )
    .unwrap()
}

fn client(port: u16, deadline: Duration) -> LocalReasoner {
    LocalReasoner::new(
        port,
        deadline,
        DeclaredArtifacts::new("a".repeat(64), "b".repeat(64)).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn actual_http_result_bridges_only_to_scoped_expiring_inference() {
    let mut server = server(
        envelope(&success().to_string()),
        StatusCode::OK,
        Duration::ZERO,
    )
    .await;
    let reasoner = client(server.port, Duration::from_secs(1));
    let input = input(draft(), BTreeSet::new());
    let result = reasoner.resolve(&input).await.unwrap();
    assert_eq!(result.proposed_entity().unwrap().as_str(), "tf1");
    let request = server.requests.recv().await.unwrap();
    let prompt: Value = serde_json::from_str(PROTOCOL).unwrap();
    assert_eq!(request["messages"][0]["content"], prompt["prompt"]);
    assert!(!request.to_string().contains("PRIVATE"));
    assert_eq!(request["max_tokens"], 512);
    let bundle = result
        .provisional(
            SourceRecordId::new("protected-derivation").unwrap(),
            100,
            60,
        )
        .unwrap();
    assert_eq!(bundle.context().record_id.as_str(), "record-PRIVATE");
    assert_eq!(bundle.context().snapshot_id.as_str(), "snapshot-PRIVATE");
    assert!(bundle
        .context()
        .evidence
        .iter()
        .all(|e| e.authority == Authority::Inference));
    let policy = Policy {
        countries: BTreeSet::new(),
        languages: BTreeSet::new(),
    };
    let assessment = bundle.assess(&policy, 101).unwrap();
    assert_eq!(assessment.fields.len(), 3);
    assert!(assessment
        .fields
        .values()
        .all(|f| f.state == FieldState::Provisional));
    assert!(bundle
        .assess(&policy, 160)
        .unwrap()
        .fields
        .values()
        .all(|f| f.state == FieldState::Unknown));
    assert!(result
        .provisional(SourceRecordId::new("p").unwrap(), u64::MAX, 1)
        .is_err());
}

#[tokio::test]
async fn abstention_keeps_explicit_quality_without_suggesting_identity() {
    let output = json!({"canonical_name":null,"candidate_id":null,"entity_type":"unknown","quality_hint":"hd"});
    let s = server(
        envelope(&output.to_string()),
        StatusCode::OK,
        Duration::ZERO,
    )
    .await;
    let result = client(s.port, Duration::from_secs(1))
        .resolve(&input(draft(), BTreeSet::new()))
        .await
        .unwrap();
    assert!(result.proposed_entity().is_none());
    let bundle = result
        .provisional(SourceRecordId::new("p").unwrap(), 1, 1)
        .unwrap();
    assert_eq!(bundle.context().evidence.len(), 1);
    assert_eq!(
        bundle.context().evidence[0].fact.field(),
        Field::QualityHint
    );
}

#[tokio::test]
async fn cannot_link_and_explicit_metadata_conflicts_reject_model_choice() {
    let s = server(
        envelope(&success().to_string()),
        StatusCode::OK,
        Duration::ZERO,
    )
    .await;
    let reasoner = client(s.port, Duration::from_secs(1));
    let denied = BTreeSet::from([EntityId::new("tf1").unwrap()]);
    assert!(matches!(
        reasoner.resolve(&input(draft(), denied)).await,
        Err(ReasonerError::Conflict)
    ));
    let mut bad = draft();
    bad.epg_id = Some("different-channel.fr".into());
    assert!(matches!(
        reasoner.resolve(&input(bad, BTreeSet::new())).await,
        Err(ReasonerError::Conflict)
    ));
    let mut bad = draft();
    bad.year = Some(1984);
    bad.candidates[0].year = Some(2021);
    assert!(matches!(
        reasoner.resolve(&input(bad, BTreeSet::new())).await,
        Err(ReasonerError::Conflict)
    ));
}

#[tokio::test]
async fn forged_fields_missing_nulls_duplicate_keys_and_inconsistent_names_fail_closed() {
    let mut extra = success();
    extra["authority"] = json!("official");
    let mut invented = success();
    invented["candidate_id"] = json!("invented");
    let mut changed = success();
    changed["canonical_name"] = json!("Other name");
    for output in [extra.to_string(), invented.to_string(), changed.to_string(),
        r#"{"entity_type":"unknown","quality_hint":"unknown"}"#.into(),
        r#"{"canonical_name":null,"candidate_id":null,"candidate_id":"tf1","entity_type":"unknown","quality_hint":"unknown"}"#.into()] {
        let s = server(envelope(&output),StatusCode::OK,Duration::ZERO).await;
        let error = client(s.port,Duration::from_secs(1)).resolve(&input(draft(),BTreeSet::new())).await.err().unwrap();
        assert_eq!(error, ReasonerError::Response);
        assert!(!error.to_string().contains("Other name"));
    }
}

#[tokio::test]
async fn busy_deadline_and_caller_cancellation_release_the_slot() {
    let mut s = server(
        envelope(&success().to_string()),
        StatusCode::OK,
        Duration::from_millis(300),
    )
    .await;
    let reasoner = Arc::new(client(s.port, Duration::from_millis(100)));
    let input = Arc::new(input(draft(), BTreeSet::new()));
    let (r, i) = (reasoner.clone(), input.clone());
    let task = tokio::spawn(async move { r.resolve(&i).await });
    s.requests.recv().await.unwrap();
    assert!(matches!(
        reasoner.resolve(&input).await,
        Err(ReasonerError::Busy)
    ));
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    assert!(matches!(
        reasoner.resolve(&input).await,
        Err(ReasonerError::Timeout)
    ));
    assert!(matches!(
        reasoner.resolve(&input).await,
        Err(ReasonerError::Timeout)
    ));
}

#[tokio::test]
async fn errors_and_oversized_responses_do_not_become_model_facts() {
    for status in [StatusCode::FOUND, StatusCode::INTERNAL_SERVER_ERROR] {
        let s = server("PRIVATE error body".into(), status, Duration::ZERO).await;
        assert!(matches!(
            client(s.port, Duration::from_secs(1))
                .resolve(&input(draft(), BTreeSet::new()))
                .await,
            Err(ReasonerError::Http(_))
        ));
    }
    let s = server(
        "x".repeat(MAX_RESPONSE_BYTES + 1),
        StatusCode::OK,
        Duration::ZERO,
    )
    .await;
    assert!(matches!(
        client(s.port, Duration::from_secs(1))
            .resolve(&input(draft(), BTreeSet::new()))
            .await,
        Err(ReasonerError::Limit)
    ));
    let truncated =
        json!({"choices":[{"finish_reason":"length","message":{"content":success().to_string()}}]});
    let s = server(truncated.to_string(), StatusCode::OK, Duration::ZERO).await;
    assert!(matches!(
        client(s.port, Duration::from_secs(1))
            .resolve(&input(draft(), BTreeSet::new()))
            .await,
        Err(ReasonerError::Response)
    ));
}

#[test]
fn request_limits_and_unknown_transport_attributes_fail_before_network() {
    let mut oversized = draft();
    oversized.title = "x".repeat(1025);
    assert!(ResolutionInput::new(
        SourceRecordId::new("r").unwrap(),
        SnapshotId::new("s").unwrap(),
        oversized,
        BTreeSet::new()
    )
    .is_err());
    assert!(serde_json::from_value::<InputDraft>(
        json!({"title":"TF1","candidates":[],"stream_url":"secret"})
    )
    .is_err());
    assert!(LocalReasoner::new(
        80,
        Duration::from_secs(1),
        DeclaredArtifacts::new("a".repeat(64), "b".repeat(64)).unwrap()
    )
    .is_err());
}

#[tokio::test]
async fn chunked_response_is_bounded_without_content_length() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 1024];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        let chunk = "x".repeat(32769);
        let response = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n8001\r\n{chunk}\r\n8001\r\n{chunk}\r\n0\r\n\r\n");
        // Early client disconnect is expected once the cap is exceeded.
        let _ = socket.write_all(response.as_bytes()).await;
    });
    assert!(matches!(
        client(port, Duration::from_secs(1))
            .resolve(&input(draft(), BTreeSet::new()))
            .await,
        Err(ReasonerError::Limit)
    ));
    task.await.unwrap();
}

#[tokio::test]
async fn provenance_fingerprint_includes_scope_and_cannot_link_constraints() {
    let s = server(envelope(&json!({"canonical_name":null,"candidate_id":null,"entity_type":"unknown","quality_hint":"unknown"}).to_string()),StatusCode::OK,Duration::ZERO).await;
    let reasoner = client(s.port, Duration::from_secs(1));
    let a = reasoner
        .resolve(&input(draft(), BTreeSet::new()))
        .await
        .unwrap();
    let b = reasoner
        .resolve(&input(
            draft(),
            BTreeSet::from([EntityId::new("tf1").unwrap()]),
        ))
        .await
        .unwrap();
    assert_ne!(a.input_fingerprint(), b.input_fingerprint());
    let other = ResolutionInput::new(
        SourceRecordId::new("other").unwrap(),
        SnapshotId::new("snapshot-PRIVATE").unwrap(),
        draft(),
        BTreeSet::new(),
    )
    .unwrap();
    let c = reasoner.resolve(&other).await.unwrap();
    assert_ne!(a.input_fingerprint(), c.input_fingerprint());
}

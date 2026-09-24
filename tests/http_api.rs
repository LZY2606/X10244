//! Black-box API checks over the real Axum router with a shared in-memory store.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use saxs_station::store::Store;
use saxs_station::web;
use serde_json::{json, Value};
use tower::ServiceExt;

fn app() -> axum::Router {
    web::app(Store::memory().unwrap())
}

async fn send(router: axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value, axum::Router) {
    let mut b = Request::builder().method(method).uri(uri);
    if body.is_some() {
        b = b.header("content-type", "application/json");
    }
    let body = match body {
        Some(v) => Body::new(v.to_string()),
        None => Body::empty(),
    };
    let resp = router.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 50_000_000).await.unwrap();
    let val = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val, router)
}

async fn setup() -> axum::Router {
    let router = app();
    let (s, v, router) = send(router, "POST", "/api/cases/load", Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["loaded"], json!(6));
    router
}

async fn normalize(router: axum::Router, raw: &str) -> (String, axum::Router) {
    let (s, v, router) = send(
        router,
        "POST",
        "/api/action",
        Some(json!({"chain":"main","action":"normalize","payload":{"raw_id": raw}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    (v["event"]["object_id"].as_str().unwrap().to_string(), router)
}

#[tokio::test]
async fn index_serves_ui_and_tour_is_available() {
    let router = app();
    let resp = router
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let (s, tour, _) = send(router, "GET", "/api/cases/tour", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(tour.as_array().unwrap().len(), 7);
}

#[tokio::test]
async fn full_chain_through_http_with_point_provenance() {
    let router = setup().await;
    let (s, state, router) = send(router, "GET", "/api/state", None).await;
    assert_eq!(s, StatusCode::OK);
    let raws: Vec<String> = state["chain"]["raws"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect();

    let (sn, router) = normalize(router, &raws[0]).await;
    let (bn, router) = normalize(router, &raws[1]).await;
    let (s, v, router) = send(
        router,
        "POST",
        "/api/action",
        Some(json!({"chain":"main","action":"subtract",
            "payload":{"sample_norm_id":sn,"buffer_norm_id":bn}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let sub = v["event"]["object_id"].as_str().unwrap().to_string();

    let (s, v, router) = send(
        router,
        "POST",
        "/api/action",
        Some(json!({"chain":"main","action":"fit_models","payload":{"sub_id":sub}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["event"]["judgment"]["status"], json!("multi"));

    // derived point traces back to both source curves
    let (s, obj, router) = send(router, "GET", &format!("/api/object/{sub}"), None).await;
    assert_eq!(s, StatusCode::OK);
    let pid = obj["object"]["sub"]["curve"]["points"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["covered"] == json!(true))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let encoded = pid.replace('@', "%40");
    let (s, point, _router) = send(router, "GET", &format!("/api/point/{encoded}"), None).await;
    eprintln!("POINT STATUS {s} BODY {point}");
    assert_eq!(s, StatusCode::OK);
    assert!(point["sample"]["raw"].is_object(), "raw source missing: {point}");
    assert!(point["sample"]["raw"]["record"]["id"].is_string());
    assert!(point["buffer"]["norm_point_ids"].is_array());
}

#[tokio::test]
async fn stale_revision_conflicts_over_http_and_export_import_works() {
    let router = setup().await;
    let (_s, state, router) = send(router, "GET", "/api/state", None).await;
    let raws: Vec<String> = state["chain"]["raws"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect();
    let (sn, router) = normalize(router, &raws[0]).await;
    let (bn, router) = normalize(router, &raws[1]).await;
    let (bn2, router) = normalize(router, &raws[2]).await;
    let _ = bn2;

    // current revision is 6 imports + 2 normalizations = 8
    let (_s, st, router) = send(router, "GET", "/api/state", None).await;
    let rev = st["chain"]["revision"].as_i64().unwrap();
    let (_s, _v, router) = send(
        router,
        "POST",
        "/api/action",
        Some(json!({"chain":"main","action":"subtract","base_revision":rev,
            "payload":{"sample_norm_id":sn,"buffer_norm_id":bn}})),
    )
    .await;
    // stale base conflicts
    let (s, v, router) = send(
        router,
        "POST",
        "/api/action",
        Some(json!({"chain":"main","action":"subtract","base_revision":rev,
            "payload":{"sample_norm_id":sn,"buffer_norm_id":bn2}})),
    )
    .await;
    eprintln!("CONF STATUS {s} BODY {v}");
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(v["conflict"], json!(true));

    // export and re-import into a fresh app
    let (s, archive, _) = send(router, "GET", "/api/export", None).await;
    assert_eq!(s, StatusCode::OK);
    let fresh = app();
    let (s, v, _fresh) = send(fresh, "POST", "/api/import", Some(archive)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["ok"], json!(true));
}

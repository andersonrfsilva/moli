use super::*;

#[tokio::test]
async fn worker_exception_constructors_do_not_invent_failure_stacks() {
    ensure_v8();
    let source = format!(
        "{}; postMessage(domExceptionStackBoundariesProbe(globalThis)); close();",
        include_str!("../../../../tests/fixtures/dom-exception-stack-boundaries.js")
    );
    let mut worker = spawn_worker(source, "https://exception-data.test/worker.js".into());
    let message = timeout(TIMEOUT, worker.recv()).await.unwrap().unwrap();
    let result: serde_json::Value = serde_json::from_str(&expect_post_json(message)).unwrap();
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(result["rows"].as_array().unwrap().len(), 15, "{result}");
}

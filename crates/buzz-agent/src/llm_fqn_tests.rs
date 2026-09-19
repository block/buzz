// Included in llm::tests to reuse the production-path HTTP capture fixture.
#[tokio::test]
async fn gpt_fqn_completion_and_summary_use_responses() {
    let response = json!({"status":"completed", "output":[{
        "type":"message", "content":[{"type":"output_text", "text":"ok"}]
    }]});
    let (base_url, captured) = spawn_sequence_stub(vec![
        StubHttpResponse::ok(response.clone()),
        StubHttpResponse::ok(response),
    ])
    .await;
    let mut config = cfg(Provider::DatabricksV2);
    config.base_url = base_url;
    config.thinking_effort = Some(ThinkingEffort::High);
    let model = "catalog.schema.goose-gpt-6-astra";
    let llm = Llm::new(&config).unwrap();
    let tools = vec![ToolDef {
        name: "test_tool".into(),
        description: "Test".into(),
        input_schema: json!({"type":"object", "properties":{}}),
    }];
    let result = llm
        .complete(
            &config,
            "system",
            &[HistoryItem::User("hello".into())],
            &tools,
            model,
        )
        .await
        .unwrap();
    assert_eq!(result.text, "ok");
    assert_eq!(
        llm.summarize(&config, "system", "history", 128, model)
            .await
            .unwrap(),
        "ok"
    );
    let requests = captured.lock().await;
    let posts: Vec<_> = requests.iter().filter(|r| r.method == "POST").collect();
    assert_eq!(posts.len(), 2);
    for request in &posts {
        assert_eq!(request.path, "/v1/ai-gateway/openai/v1/responses");
        let body = request.body.as_ref().unwrap();
        assert_eq!(body["model"], model);
        assert!(body.get("input").is_some());
        assert!(body.get("messages").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }
    let completion = posts[0].body.as_ref().unwrap();
    assert!(completion["input"].is_array());
    assert_eq!(completion["reasoning"]["effort"], "high");
    assert_eq!(completion["tools"][0]["type"], "function");
    assert_eq!(completion["tools"][0]["name"], "test_tool");
    assert_eq!(posts[1].body.as_ref().unwrap()["max_output_tokens"], 128);
}

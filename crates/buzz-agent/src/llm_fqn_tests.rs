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

const OPUS_UC: &str = "data_workflow_tools.goose.goose-claude-opus-5-5";

#[tokio::test]
async fn opus_uc_replays_ordered_signed_content_with_high_effort() {
    // Empty thinking can still carry a signature; redacted data is opaque.
    let blocks = json!([
        {"type":"thinking", "thinking":"", "signature":"signed-empty"},
        {"type":"tool_use", "id":"a", "name":"test_tool", "input":{"n":1}},
        {"type":"redacted_thinking", "data":"opaque-redacted"},
        {"type":"tool_use", "id":"b", "name":"test_tool", "input":{"n":2}},
        {"type":"thinking", "thinking":"checking", "signature":"signed-check"},
        {"type":"text", "text":"running both"}
    ]);
    let done = json!({"stop_reason":"end_turn", "content":[{"type":"text","text":"done"}]});
    let (base_url, captured) = spawn_sequence_stub(vec![
        StubHttpResponse::ok(json!({"stop_reason":"tool_use", "content":blocks})),
        StubHttpResponse::ok(done.clone()),
        StubHttpResponse::ok(done),
    ]).await;
    let mut config = cfg(Provider::DatabricksV2);
    config.base_url = base_url;
    config.thinking_effort = Some(ThinkingEffort::High);
    config.prompt_caching = true;
    let llm = Llm::new(&config).unwrap();
    let tools = vec![ToolDef { name:"test_tool".into(), description:"Test".into(),
        input_schema:json!({"type":"object", "properties":{"n":{"type":"integer"}}}) }];
    let mut history = vec![HistoryItem::User("run both".into())];
    let first = llm.complete(&config, "system", &history, &tools, OPUS_UC).await.unwrap();
    assert_eq!(first.stop, ProviderStop::ToolUse);
    assert_eq!(first.reasoning, "checking");
    assert_eq!(first.tool_calls.len(), 2);
    assert_eq!(first.tool_calls[1].arguments, json!({"n":2}));
    history.push(HistoryItem::Assistant { text:first.text, tool_calls:first.tool_calls,
        reasoning_details:first.reasoning_details });
    for id in ["a", "b"] {
        history.push(HistoryItem::ToolResult(crate::types::ToolResult {
            provider_id:id.into(), content:vec![ToolResultContent::Text(format!("result-{id}"))], is_error:false,
        }));
    }
    let second = llm.complete(&config, "system", &history, &tools, OPUS_UC).await.unwrap();
    assert_eq!(second.stop, ProviderStop::EndTurn);
    assert_eq!(second.text, "done");
    assert_eq!(llm.summarize(&config, "system", "history", 128, OPUS_UC).await.unwrap(), "done");
    let requests = captured.lock().await;
    let posts: Vec<_> = requests.iter().filter(|r| r.method == "POST").collect();
    assert_eq!(posts.len(), 3);
    for post in &posts {
        assert_eq!(post.path, "/v1/ai-gateway/anthropic/v1/messages");
        assert_eq!(post.body.as_ref().unwrap()["model"], OPUS_UC);
        assert!(post.body.as_ref().unwrap().get("reasoning_effort").is_none());
    }
    for post in &posts[..2] {
        let body = post.body.as_ref().unwrap();
        assert_eq!(body["output_config"], json!({"effort":"high"}));
        assert_eq!(body["thinking"], json!({"type":"adaptive", "display":"summarized"}));
        assert_eq!(body["tools"][0]["input_schema"], tools[0].input_schema);
    }
    let body = posts[1].body.as_ref().unwrap();
    let mut replay = body["messages"][1]["content"].clone();
    assert_eq!(replay[5]["cache_control"], json!({"type":"ephemeral"}));
    replay[5].as_object_mut().unwrap().remove("cache_control");
    assert_eq!(replay, blocks);
    let results = body["messages"][2]["content"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    for (result, id) in results.iter().zip(["a", "b"]) {
        assert_eq!(result["type"], "tool_result");
        assert_eq!(result["tool_use_id"], id);
        assert_eq!(result["content"][0]["text"], format!("result-{id}"));
    }
}

#[tokio::test]
async fn neighboring_opus_uc_services_stay_on_mlflow_chat() {
    for model in ["system.ai.claude-opus-5-5", "other.goose.goose-claude-opus-5-5",
        "data_workflow_tools.goose.goose-claude-opus-5-5-preview"] {
        let (base_url, captured) = spawn_sequence_stub(vec![StubHttpResponse::ok(json!({
            "choices":[{"finish_reason":"stop", "message":{"content":"ok"}}]
        }))]).await;
        let mut config = cfg(Provider::DatabricksV2);
        config.base_url = base_url;
        config.thinking_effort = Some(ThinkingEffort::High);
        Llm::new(&config).unwrap().complete(&config, "system", &[HistoryItem::User("hi".into())], &[], model).await.unwrap();
        let requests = captured.lock().await;
        let post = requests.iter().find(|r| r.method == "POST").unwrap();
        assert_eq!(post.path, "/v1/ai-gateway/mlflow/v1/chat/completions");
        let body = post.body.as_ref().unwrap();
        assert_eq!(body["model"], model);
        assert_eq!(body["reasoning_effort"], "high");
        assert!(body.get("thinking").is_none());
    }
}

#[test]
fn native_thinking_tail_is_not_cache_stamped_or_leaked_to_chat() {
    for block in [json!({"type":"thinking", "thinking":"", "signature":"signed"}),
        json!({"type":"redacted_thinking", "data":"opaque"})] {
        let response = parse_anthropic(json!({"stop_reason":"end_turn", "content":[block]})).unwrap();
        let history = vec![HistoryItem::Assistant { text:response.text, tool_calls:response.tool_calls,
            reasoning_details:response.reasoning_details }];
        let mut config = cfg(Provider::DatabricksV2);
        config.prompt_caching = true;
        let native = anthropic_body(&config, "system", &history, &[], OPUS_UC, Some(ThinkingEffort::High), "databricks_v2");
        assert_eq!(native["messages"][0]["content"], json!([block]));
        let chat = openai_body(&config, "system", &history, &[], "other", None);
        assert!(chat["messages"][1].get("reasoning_details").is_none());
        assert!(!responses_body(&config, "system", &history, &[], "other", None).to_string().contains("anthropic_content"));
    }
}

use kiroku_core::translate::{AiCfg, Translator};
use serde_json::{json, Value};

#[test]
fn legacy_credentials_survive_migration_and_roundtrip() {
    let cfg: AiCfg = serde_json::from_value(json!({
        "enabled":true,"api_key":"synthetic","model":"custom-model",
        "base_url":"https://gateway.example/anthropic","effort":"medium","title":false,"plot":true
    })).unwrap();
    let value = serde_json::to_value(&cfg).unwrap();
    assert_eq!(value["providers"][0]["api_key"], "synthetic");
    assert_eq!(value["providers"][0]["model"], "custom-model");
    assert_eq!(value["providers"][0]["protocol"], "anthropic");
    assert_eq!(value["providers"][0]["effort"], "medium");
    assert_eq!(value["providers"][0]["base_url"], "https://gateway.example/anthropic");
    assert_eq!(value["selected_provider"], value["providers"][0]["id"]);
    assert_eq!(value["title"], false);
    assert!(value.get("api_key").is_none());
    assert_eq!(serde_json::from_value::<AiCfg>(value).unwrap(), cfg);
}

#[test]
fn deleted_selection_does_not_fall_back_to_another_provider() {
    let cfg: AiCfg = serde_json::from_value(json!({"enabled":true,"providers":[{
        "id":"other","name":"Other","api_key":"synthetic"
    }],"selected_provider":"deleted"})).unwrap();
    assert!(Translator::new(cfg).is_err(), "must not silently choose another provider");
    let cfg: AiCfg = serde_json::from_value(json!({"providers":[],"selected_provider":null})).unwrap();
    assert_eq!(serde_json::to_value(&cfg).unwrap()["providers"], json!([]));
    assert!(Translator::new(cfg).is_err());
}

// Captures the real Translator HTTP request; only the remote API is replaced.
async fn call(protocol: &str, suffix: &str, response: Value) -> (Result<kiroku_core::translate::Translated,String>, String, Value) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}{}", listener.local_addr().unwrap(), suffix);
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 8192];
        let end = loop {
            let n = stream.read(&mut buffer).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buffer[..n]);
            let text = String::from_utf8_lossy(&bytes);
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text[..end].lines().find_map(|l| l.to_lowercase().strip_prefix("content-length: ").and_then(|s|s.parse::<usize>().ok())).unwrap();
                if bytes.len() >= end + 4 + length { break end; }
            }
        };
        let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
        let request: Value = serde_json::from_slice(&bytes[end+4..]).unwrap();
        let body = response.to_string();
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        (headers, request)
    });
    // Legacy fields make the pre-feature implementation send to the fake endpoint too.
    let cfg: AiCfg = serde_json::from_value(json!({"enabled":true,"api_key":"synthetic","base_url":address,
        "selected_provider":"custom","providers":[{"id":"custom","name":"Custom","protocol":protocol,
        "api_key":"synthetic","base_url":address,"model":"test-model","effort":"","output_format":"json_schema"}]})).unwrap();
    let result = Translator::new(cfg).unwrap().translate(Some("海辺の町"), None).await;
    let (headers, request) = worker.join().unwrap();
    (result, headers, request)
}

#[tokio::test]
async fn openai_uses_chat_endpoint_bearer_auth_and_structured_output() {
    let (result, headers, request) = call("openai", "/gateway/v1/", json!({"choices":[{
        "finish_reason":"stop","message":{"content":"{\"title\":\"海边小镇\",\"plot\":\"\"}"}
    }]})).await;
    assert_eq!(result.unwrap().title.as_deref(), Some("海边小镇"));
    assert!(headers.starts_with("POST /gateway/v1/chat/completions "));
    assert!(headers.contains("authorization: Bearer synthetic"));
    assert!(!headers.contains("x-api-key"));
    assert_eq!(request["model"], "test-model");
    assert_eq!(request["response_format"]["type"], "json_schema");
    assert_eq!(request["messages"][0]["role"], "system");
    assert!(request.get("fallbacks").is_none());
    assert!(request.get("output_config").is_none());
    assert!(request.get("reasoning_effort").is_none());
}

#[tokio::test]
async fn anthropic_uses_messages_and_no_extensions_by_default() {
    let (result, headers, request) = call("anthropic", "/gateway", json!({"stop_reason":"end_turn","content":[{
        "type":"text","text":"{\"title\":\"海边小镇\",\"plot\":\"\"}"
    }]})).await;
    assert!(result.is_ok());
    assert!(headers.starts_with("POST /gateway/v1/messages "));
    assert!(headers.contains("x-api-key: synthetic"));
    assert!(headers.contains("anthropic-version: 2023-06-01"));
    assert!(!headers.contains("authorization:"));
    assert!(!headers.contains("anthropic-beta:"));
    assert!(request.get("fallbacks").is_none());
    assert!(request["output_config"].get("effort").is_none());
}

#[tokio::test]
async fn openai_rejects_truncation_refusal_and_invalid_translation_shape() {
    for response in [
        json!({"choices":[{"finish_reason":"length","message":{"content":"{\"title\":\"partial\",\"plot\":\"\"}"}}]}),
        json!({"choices":[{"finish_reason":"stop","message":{"refusal":"declined","content":"{\"title\":\"x\",\"plot\":\"\"}"}}]}),
        json!({"choices":[{"finish_reason":"stop","message":{"content":"{}"}}]}),
    ] {
        assert!(call("openai", "", response).await.0.is_err());
    }
}

#[test]
fn compatible_mode_omits_optional_parameters_for_both_protocols() {
    use kiroku_core::translate::{AiProvider, AiProtocol, OutputFormat, build_request};
    for protocol in [AiProtocol::Openai, AiProtocol::Anthropic] {
        let p = AiProvider { protocol, output_format: OutputFormat::Prompt, ..Default::default() };
        let request = build_request(&p, Some("海辺の町"), None);
        assert!(request.get("response_format").is_none());
        assert!(request.get("output_config").is_none());
        assert!(request.get("reasoning_effort").is_none());
        assert!(request.get("fallbacks").is_none());
        assert!(request.to_string().contains("title"));
        let configured = AiProvider { effort:"medium".into(), ..p };
        let request = build_request(&configured, None, Some("町の物語"));
        match protocol {
            AiProtocol::Openai => assert_eq!(request["reasoning_effort"], "medium"),
            AiProtocol::Anthropic => assert_eq!(request["output_config"]["effort"], "medium"),
        }
    }
}

#[test]
fn endpoint_normalizes_version_and_rejects_credentials_and_query_parameters() {
    use kiroku_core::translate::{AiProtocol, AiProvider};
    for base in ["https://service.example/gateway", "https://service.example/gateway/v1/", "https://service.example/gateway/v1/chat/completions"] {
        let p = AiProvider { protocol: AiProtocol::Openai, base_url: base.into(), ..Default::default() };
        assert_eq!(p.endpoint().unwrap().as_str(), "https://service.example/gateway/v1/chat/completions");
    }
    for base in ["file:///tmp/key", "https://user:password@service.example", "https://service.example?key=secret", "https://service.example#fragment"] {
        assert!(AiProvider {base_url:base.into(),..Default::default()}.endpoint().is_err());
    }
}

#[test]
fn invalid_selected_provider_is_rejected_when_saving_settings() {
    use kiroku_core::settings::Settings;
    let mut settings = Settings::default();
    settings.ai.enabled = true;
    assert!(settings.validate().unwrap_err().contains("API Key"));
    settings.ai.providers[0].api_key = "synthetic".into();
    assert!(settings.validate().is_ok());
    settings.ai.providers.push(settings.ai.providers[0].clone());
    assert!(settings.validate().unwrap_err().contains("重复"));
}

#[test]
fn parsers_reject_incomplete_and_malformed_responses_even_when_text_looks_valid() {
    use kiroku_core::translate::{AiProtocol, parse_response};
    for text in ["{}", "[]", "null", "{\"title\":42,\"plot\":\"\"}", "{\"title\":\"ok\"}", "not json"] {
        for protocol in [AiProtocol::Openai, AiProtocol::Anthropic] {
            let v = match protocol {
                AiProtocol::Openai => json!({"choices":[{"finish_reason":"stop","message":{"content":text}}]}),
                AiProtocol::Anthropic => json!({"stop_reason":"end_turn","content":[{"type":"text","text":text}]}),
            };
            assert!(parse_response(protocol, &v).is_err());
        }
    }
    let content = "{\"title\":\"海边小镇\",\"plot\":\"\"}";
    for reason in ["tool_use", "pause_turn", "max_tokens", "refusal", "unknown"] {
        assert!(parse_response(AiProtocol::Anthropic, &json!({"stop_reason":reason,"content":[{"type":"text","text":content}]})).is_err());
    }
    for reason in ["length", "tool_calls", "content_filter", "unknown"] {
        assert!(parse_response(AiProtocol::Openai, &json!({"choices":[{"finish_reason":reason,"message":{"content":content}}]})).is_err());
    }
    assert_eq!(parse_response(AiProtocol::Openai, &json!({"choices":[{"finish_reason":"stop","message":{"content":format!("```json\n{content}\n```")}}]})).unwrap().title.as_deref(), Some("海边小镇"));
}

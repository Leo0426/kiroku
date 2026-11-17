use kiroku_core::{names::{NameRule, NameRules}, translate::{AiCfg, NameKind, suggest_name}};
use serde_json::{json, Value};

// Captures the real Translator HTTP request; only the remote API is replaced.
async fn call(protocol: &str, suffix: &str, response: Value) -> (Result<NameRule,String>, String, Value) {
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
    let result = suggest_name(cfg, &NameRules::default(), NameKind::Series, "海辺の町").await;
    let (headers, request) = worker.join().unwrap();
    (result, headers, request)
}


#[tokio::test]
async fn candidate_preserves_source_and_uses_selected_protocol() {
    for protocol in ["openai", "anthropic"] {
        let content=json!({"canonical":"海边小镇"}).to_string();
        let response=if protocol=="openai" {json!({"choices":[{"finish_reason":"stop","message":{"content":content}}]})}
            else {json!({"stop_reason":"end_turn","content":[{"type":"text","text":content}]})};
        let (result,headers,request)=call(protocol,"/gateway",response).await;
        assert_eq!(result.unwrap(),NameRule{canonical:"海边小镇".into(),aliases:vec!["海辺の町".into()]});
        let (input,schema)=if protocol=="openai" {
            assert!(headers.contains("authorization: Bearer synthetic"));
            (&request["messages"][1]["content"], &request["response_format"]["json_schema"]["schema"])
        } else {
            assert!(headers.contains("x-api-key: synthetic"));
            (&request["messages"][0]["content"], &request["output_config"]["format"]["schema"])
        };
        assert_eq!(serde_json::from_str::<Value>(input.as_str().unwrap()).unwrap()["source"],"海辺の町");
        assert_eq!(schema["required"],json!(["canonical"]));
    }
}

#[tokio::test]
async fn bad_outputs_and_refusals_never_become_rules() {
    for response in [
        json!({"choices":[{"finish_reason":"length","message":{"content":"{\"canonical\":\"部分\"}"}}]}),
        json!({"choices":[{"finish_reason":"stop","message":{"refusal":"refused","content":"{\"canonical\":\"拒绝\"}"}}]}),
        json!({"choices":[{"finish_reason":"stop","message":{"content":"{\"canonical\":\" \"}"}}]}),
        json!({"choices":[{"finish_reason":"stop","message":{"content":"{\"canonical\":123}"}}]}),
        json!({"choices":[{"finish_reason":"stop","message":{"content":"{\"canonical\":\"名字\",\"extra\":true}"}}]}),
    ] { assert!(call("openai","",response).await.0.is_err()); }
    assert!(call("anthropic","",json!({"stop_reason":"refusal","content":[]})).await.0.is_err());
}

#[tokio::test]
async fn dictionary_precedes_ai_and_known_names_are_not_retranslated() {
    let preset=kiroku_core::names::tag_preset();
    let rule=preset.iter().find(|r|!r.aliases.is_empty()).unwrap();
    let suggestion=suggest_name(AiCfg::default(),&NameRules::default(),NameKind::Tags,&rule.aliases[0]).await.unwrap();
    assert_eq!(suggestion.canonical,rule.canonical);
    assert_eq!(suggestion.aliases,vec![rule.aliases[0].clone()]);
    let rules=NameRules{series:vec![NameRule{canonical:"旅行".into(),aliases:vec!["たび".into()]}],..Default::default()};
    let before=rules.clone();
    for source in ["たび","旅行"] {
        assert!(suggest_name(AiCfg::default(),&rules,NameKind::Series,source).await.unwrap_err().contains("已有规则"));
    }
    assert_eq!(rules,before);
    assert!(suggest_name(AiCfg::default(),&rules,NameKind::Series,"").await.is_err());
    assert!(suggest_name(AiCfg::default(),&rules,NameKind::Series,"新しい旅").await.unwrap_err().contains("启用"));
}

use super::{AiProtocol, AiProvider, OutputFormat, Translated};
use serde_json::{json, Value};

const SYSTEM: &str = "你是专业的日译中翻译。把用户给出的日本影片标题和简介翻译成自然、通顺的简体中文。\
要求:忠实原意,不增删内容,不加解释或注释;人名、编号、厂牌名保留原文;\
只返回 JSON 对象,包含 title 和 plot 两个字符串字段,不要 Markdown 代码块。输入为空的字段输出空字符串。";

pub fn build_request(cfg: &AiProvider, title: Option<&str>, plot: Option<&str>) -> Value {
    let input = json!({"title": title.unwrap_or(""), "plot": plot.unwrap_or("")});
    let content = format!("请翻译以下 JSON 中的字段:\n{input}");
    let schema = json!({"type":"object","properties":{
        "title":{"type":"string"},"plot":{"type":"string"}
    },"required":["title","plot"],"additionalProperties":false});
    match cfg.protocol {
        AiProtocol::Anthropic => {
            let mut r = json!({"model":cfg.model.trim(),"max_tokens":16000,"system":SYSTEM,
                "messages":[{"role":"user","content":content}]});
            if cfg.anthropic_fallback { r["fallbacks"] = json!("default"); }
            if !cfg.effort.is_empty() { r["output_config"]["effort"] = json!(cfg.effort); }
            if cfg.output_format == OutputFormat::JsonSchema {
                r["output_config"]["format"] = json!({"type":"json_schema","schema":schema});
            }
            r
        }
        AiProtocol::Openai => {
            let mut r = json!({"model":cfg.model.trim(),"messages":[
                {"role":"system","content":SYSTEM},{"role":"user","content":content}]});
            // Leave the output limit at the provider default for compatibility with models
            // that accept either max_tokens or max_completion_tokens, but not both.
            if !cfg.effort.is_empty() { r["reasoning_effort"] = json!(cfg.effort); }
            if cfg.output_format == OutputFormat::JsonSchema {
                r["response_format"] = json!({"type":"json_schema","json_schema":{
                    "name":"translation","strict":true,"schema":schema}});
            }
            r
        }
    }
}

pub fn parse_response(protocol: AiProtocol, v: &Value) -> Result<Translated, String> {
    let text = match protocol {
        AiProtocol::Anthropic => {
            match v["stop_reason"].as_str() {
                Some("end_turn") => {},
                Some("refusal") => return Err("模型拒绝翻译这条内容".into()),
                Some("max_tokens") => return Err("输出被截断(max_tokens)".into()),
                _ => return Err("AI 响应未正常完成".into()),
            }
            v["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str()).collect::<String>()
        }
        AiProtocol::Openai => {
            let choice = &v["choices"][0];
            let message = &choice["message"];
            if message.get("refusal").is_some_and(|r| !r.is_null() && r.as_str() != Some("")) {
                return Err("模型拒绝翻译这条内容".into());
            }
            match choice["finish_reason"].as_str() {
                Some("stop") => {},
                Some("length") => return Err("输出被截断(length)".into()),
                Some("content_filter") => return Err("模型拒绝翻译这条内容".into()),
                _ => return Err("AI 响应未正常完成".into()),
            }
            message["content"].as_str().unwrap_or_default().to_string()
        }
    };
    if text.trim().is_empty() { return Err("模型没有返回译文".into()); }
    let text = text.trim();
    // Prompt-only compatible services sometimes wrap their valid JSON in a fence.
    let text = text.strip_prefix("```json").or_else(|| text.strip_prefix("```"))
        .and_then(|t| t.trim_end().strip_suffix("```" )).unwrap_or(text).trim();
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Output { title: String, plot: String }
    let out: Output = serde_json::from_str(text).map_err(|_| "译文必须为包含 title、plot 字符串的 JSON 对象")?;
    let pick = |s: String| { let s = s.trim(); if s.is_empty() { None } else { Some(s.to_string()) } };
    Ok(Translated { title: pick(out.title), plot: pick(out.plot) })
}

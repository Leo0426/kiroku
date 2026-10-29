use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProtocol { #[default] Anthropic, Openai }

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat { #[default] JsonSchema, Prompt }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiProvider {
    pub id: String,
    pub name: String,
    pub protocol: AiProtocol,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    /// Empty means the model's default; omit the parameter for compatible services.
    pub effort: String,
    pub output_format: OutputFormat,
    /// Anthropic-specific extension, opt-in for custom providers.
    pub anthropic_fallback: bool,
}

impl Default for AiProvider {
    fn default() -> Self {
        Self { id: "anthropic".into(), name: "Anthropic".into(), protocol: AiProtocol::Anthropic,
            api_key: String::new(), model: "claude-opus-5-5".into(), base_url: "https://api.anthropic.com".into(),
            effort: String::new(), output_format: OutputFormat::JsonSchema, anthropic_fallback: false }
    }
}

impl AiProvider {
    pub fn endpoint(&self) -> Result<reqwest::Url, String> {
        let mut url = reqwest::Url::parse(self.base_url.trim()).map_err(|_| "AI API 地址无效")?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || !url.username().is_empty()
            || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
            return Err("AI API 地址须为不含账号、查询参数或片段的 http(s) 基础地址".into());
        }
        let path = url.path().trim_end_matches('/');
        // Accept a full endpoint as well as a versioned or unversioned base.
        let suffix = match self.protocol { AiProtocol::Anthropic => "/messages", AiProtocol::Openai => "/chat/completions" };
        let path = if path.ends_with(suffix) { path.to_string() }
            else if path.ends_with("/v1") { format!("{path}{suffix}") }
            else { format!("{path}/v1{suffix}") };
        url.set_path(&path);
        Ok(url)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.endpoint()?;
        if self.api_key.trim().is_empty() { return Err("请填写 AI Provider 的 API Key".into()); }
        if self.model.trim().is_empty() { return Err("请填写 AI Provider 的模型".into()); }
        if !matches!(self.effort.as_str(), "" | "low" | "medium" | "high") { return Err("AI 思考强度无效".into()); }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AiCfg {
    pub enabled: bool,
    pub providers: Vec<AiProvider>,
    pub selected_provider: Option<String>,
    pub title: bool,
    pub plot: bool,
}

impl Default for AiCfg {
    fn default() -> Self {
        Self { enabled: false, providers: vec![AiProvider { effort: "low".into(), anthropic_fallback: true, ..Default::default() }],
            selected_provider: Some("anthropic".into()), title: true, plot: true }
    }
}

// Old single-provider settings are accepted only on input, never written back.
impl<'de> Deserialize<'de> for AiCfg {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(default)]
        struct Wire {
            enabled: bool, title: bool, plot: bool,
            providers: Option<Vec<AiProvider>>, selected_provider: Option<String>,
            api_key: String, model: String, base_url: String, effort: String,
        }
        impl Default for Wire {
            fn default() -> Self {
                let p = AiProvider::default();
                Self { enabled: false, title: true, plot: true, providers: None, selected_provider: None,
                    api_key: p.api_key, model: p.model, base_url: p.base_url, effort: "low".into() }
            }
        }
        let w = Wire::deserialize(deserializer)?;
        let (providers, selected_provider) = match w.providers {
            Some(providers) => (providers, w.selected_provider),
            None => {
                let fallback = w.base_url.trim().trim_end_matches('/') == "https://api.anthropic.com";
                let p = AiProvider { api_key: w.api_key, model: w.model, base_url: w.base_url,
                    effort: w.effort, anthropic_fallback: fallback, ..Default::default() };
                let id = p.id.clone();
                (vec![p], Some(id))
            }
        };
        Ok(Self { enabled: w.enabled, title: w.title, plot: w.plot, providers, selected_provider })
    }
}

impl AiCfg {
    pub fn selected(&self) -> Result<&AiProvider, String> {
        self.selected_provider.as_ref().and_then(|id| self.providers.iter().find(|p| &p.id == id))
            .ok_or_else(|| "请选择用于汉化的 AI Provider".into())
    }

    pub fn usable(&self) -> bool { self.enabled && (self.title || self.plot) }

    pub fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::HashSet::new();
        for p in &self.providers {
            if p.id.trim().is_empty() || !ids.insert(&p.id) { return Err("AI Provider ID 为空或重复".into()); }
            if p.name.trim().is_empty() { return Err("AI Provider 名称不能为空".into()); }
        }
        if self.enabled { self.selected()?.validate()?; }
        Ok(())
    }
}

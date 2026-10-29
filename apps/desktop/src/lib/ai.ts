import type { AiCfg, AiProvider, AiProtocol } from './types'

export function newAiProvider(kind: AiProtocol | 'custom'): AiProvider {
  return {
    id: crypto.randomUUID(), name: kind === 'custom' ? '自定义 Provider' : kind === 'openai' ? 'OpenAI' : 'Anthropic',
    protocol: kind === 'anthropic' ? 'anthropic' : 'openai', api_key: '',
    base_url: kind === 'custom' ? '' : kind === 'openai' ? 'https://api.openai.com/v1' : 'https://api.anthropic.com',
    model: kind === 'custom' ? '' : kind === 'openai' ? 'gpt-4.1-mini' : 'claude-opus-5-5',
    effort: kind === 'anthropic' ? 'low' : '', output_format: kind === 'custom' ? 'prompt' : 'json_schema',
    anthropic_fallback: kind === 'anthropic',
  }
}

/** Mirror Rust AiCfg deserialization, including old browser-preview settings. */
export function normalizeAi(raw: Partial<AiCfg> & Partial<Pick<AiProvider, 'api_key' | 'model' | 'base_url' | 'effort'>> = {}): AiCfg {
  const base = { enabled: raw.enabled ?? false, title: raw.title ?? true, plot: raw.plot ?? true }
  if (Array.isArray(raw.providers)) return { ...base, providers: raw.providers, selected_provider: raw.selected_provider ?? null }
  const p = newAiProvider('anthropic')
  p.id = 'anthropic'
  p.api_key = raw.api_key ?? p.api_key
  p.model = raw.model ?? p.model
  p.base_url = raw.base_url ?? p.base_url
  p.effort = raw.effort ?? p.effort
  p.anthropic_fallback = p.base_url.trim().replace(/\/+$/, '') === 'https://api.anthropic.com'
  return { ...base, providers: [p], selected_provider: p.id }
}

export function selectedAiProvider(cfg: AiCfg): AiProvider | undefined {
  return cfg.providers.find(p => p.id === cfg.selected_provider)
}

export function validateAiProvider(p: AiProvider | undefined): asserts p is AiProvider {
  if (!p) throw '请选择用于汉化的 AI Provider'
  let url: URL
  try { url = new URL(p.base_url.trim()) } catch { throw 'AI API 地址无效' }
  if (!['http:', 'https:'].includes(url.protocol) || !url.hostname || url.username || url.password || url.search || url.hash) {
    throw 'AI API 地址须为不含账号、查询参数或片段的 http(s) 基础地址'
  }
  if (!p.api_key.trim()) throw '请填写 AI Provider 的 API Key'
  if (!p.model.trim()) throw '请填写 AI Provider 的模型'
  if (!['', 'low', 'medium', 'high'].includes(p.effort)) throw 'AI 思考强度无效'
}

export function validateAi(cfg: AiCfg) {
  const ids = new Set<string>()
  for (const p of cfg.providers) {
    if (!p.id.trim() || ids.has(p.id)) throw 'AI Provider ID 为空或重复'
    ids.add(p.id)
    if (!p.name.trim()) throw 'AI Provider 名称不能为空'
  }
  if (cfg.enabled) validateAiProvider(selectedAiProvider(cfg))
}

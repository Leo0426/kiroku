import type { SourceCheck } from './types'

const messages: Record<string, string> = {
  ok: '站点可达',
  authenticated: '站点可达，已登录',
  unauthenticated: '站点可达，未登录，请填写或更新 Cookie',
  authUnknown: '站点可达，登录状态未确认',
  needLogin: '站点要求登录，请核对 Cookie',
  invalidUrl: '站点地址无效，请核对域名',
  invalidProxy: '代理配置无效，请核对代理地址',
  connectionFailed: '无法建立连接，请检查网络和本站的代理设置',
  timeout: '请求超时，请稍后重试',
  certificateError: '无法验证 HTTPS 证书，请检查系统时间和证书配置',
  tlsError: 'HTTPS 握手失败',
  proxyAuthRequired: '代理服务器要求认证',
  redirectError: '页面跳转失败',
  unexpectedOrigin: '检测页面跳转到了其他站点',
  responseReadFailed: '未能读取完整的站点响应',
  upstreamError: '站点服务暂时不可用',
  upstreamTlsError: '站点与上游服务器的 HTTPS 连接异常',
  httpError: '站点返回异常状态',
  network: '网络请求未完成',
  blocked: '站点拦截了请求',
  geoBlocked: '站点限制当前地区访问',
  rateLimited: '请求过于频繁，请稍后重试',
  unexpectedResponse: '站点响应内容与预期不符',
  endpointNotFound: '检测页面不存在',
  disabledOrUnknown: '站点未启用或不存在',
  cancelled: '检测已取消',
  undeclared: '该站点暂不支持检测',
}

export function sourceCheckMessage({ sourceId, outcome }: SourceCheck): string {
  const text = messages[outcome.reason] ?? '未能确认检测结果'
  const status = outcome.httpStatus ? ` · HTTP ${outcome.httpStatus}` : ''
  const unverified = sourceId === 'javdb' && outcome.status === 'failed' && outcome.reason !== 'needLogin'
  return text + status + (unverified ? '。尚未验证 Cookie。' : '')
}

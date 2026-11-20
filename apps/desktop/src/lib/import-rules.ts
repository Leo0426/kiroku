import { validateTemplate } from './template-preview'
import type { ImportRules, Settings } from './types'

export function defaultImportRules(s: Settings): ImportRules {
  return { mode: s.organize_mode, directoryTemplate: s.dir_template, fileTemplate: s.file_template, plexNaming: s.plex.naming, subdirectory: '' }
}

export function checkImportName(name: string) {
  const reserved = /^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)/i.test(name)
  if (!name || name !== name.trim().replace(/\s+/gu, ' ') || name.startsWith('.') || name.endsWith('.') || /[\\/:*?"<>|{}\[\]\u0000-\u001f]/.test(name)
    || reserved || new TextEncoder().encode(name).length > 160) {
    throw new Error('请使用普通名称，去掉路径分隔符、占位符或保留字符')
  }
}

export function projectImportRules(s: Settings, rules: ImportRules): Settings {
  if (rules.subdirectory) rules.subdirectory.split('/').forEach(checkImportName)
  if (!rules.directoryTemplate.trim()) throw new Error('请填写目录模板')
  if (rules.directoryTemplate !== s.dir_template) validateTemplate(rules.directoryTemplate, false)
  if (rules.fileTemplate !== s.file_template) validateTemplate(rules.fileTemplate, true)
  return {
    ...s, organize_mode: rules.mode, plex: { ...s.plex, naming: rules.plexNaming },
    dir_template: [rules.subdirectory, rules.directoryTemplate].filter(Boolean).join('/'),
    file_template: rules.fileTemplate,
  }
}

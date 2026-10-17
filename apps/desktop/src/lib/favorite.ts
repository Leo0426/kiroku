import { api } from './api'

/** 收藏只写 sidecar,走可撤销修订:预览后直接保存(没有需要人工确认的差异)。 */
export async function setFavorite(code: string, favorite: boolean): Promise<void> {
  const preview = await api.metadataUpdatePreview(code, { kind: 'favorite', favorite })
  const saved = await api.revisionSave(preview.token, preview.root, preview.code)
  if (saved.error) throw saved.error
  if (saved.indexError) throw `收藏已保存，但索引更新失败：${saved.indexError}`
}

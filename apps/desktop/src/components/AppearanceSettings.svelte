<script lang="ts">
  import { appearance, type Appearance } from '../lib/appearance.svelte'
  import { app } from '../lib/state.svelte'
  function update(patch: Partial<Appearance>) { if (!appearance.update(patch)) app.toast('error', '外观已应用，但无法保存到本机。请检查存储权限。') }
</script>
<section class="card" aria-label="外观偏好">
  <h2>外观</h2>
  <p class="muted">立即生效，仅保存在这台设备。</p>
  <label>主题<select aria-label="主题" id="appearance-theme" class="input" value={appearance.value.theme} onchange={e => update({ theme: e.currentTarget.value as Appearance['theme'] })}><option value="system">跟随系统</option><option value="light">浅色</option><option value="dark">深色</option></select></label>
  <label>文字大小<select aria-label="文字大小" id="appearance-text" class="input" value={appearance.value.textScale} onchange={e => update({ textScale: Number(e.currentTarget.value) as Appearance['textScale'] })}><option value={100}>标准 · 100%</option><option value={115}>较大 · 115%</option><option value={130}>大 · 130%</option></select></label>
  <label>动态效果<select aria-label="动态效果" id="appearance-motion" class="input" value={appearance.value.motion} onchange={e => update({ motion: e.currentTarget.value as Appearance['motion'] })}><option value="system">跟随系统</option><option value="reduced">减少动态效果</option></select></label>
</section>
<style>
  section { padding:var(--s5); display:grid; gap:var(--s3); }
  h2 { font-size:var(--fs-lg); }
  label { display:flex; align-items:center; flex-wrap:wrap; gap:var(--s3); justify-content:space-between; }
  select { min-width:150px; }
</style>

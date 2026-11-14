<script lang="ts">
  import Art from '../components/Art.svelte';
  import { S } from '../lib/strings';
  import { failureText } from '../lib/ipc';
  import { msgIn } from '../lib/motion';
  import { store } from '../lib/store.svelte';

  let error = $state<string | null>(null);

  // Nothing to ask (decision 2026-11-14: the installer adds the Windows firewall rule, start
  // at login is a setting, on by default): finish setup and go straight to adding a device.
  // The permissions screen shows only when the OS blocks something (macOS rows).
  // `from` is the button: the S1 shape grows out of it (src/lib/flow.ts).
  async function start(from: Element) {
    error = null;
    if (store.needsPerms) {
      store.go('perms', { from });
      return;
    }
    try {
      await store.api!.call('setup.step', { row: 'done', answer: true });
      await store.refresh();
      store.go('add', { from });
    } catch (e) {
      error = failureText(e);
    }
  }
</script>

<div style="flex: 1; display: flex; flex-direction: column; padding: 84px 28px 28px 28px;">
  <div data-m="text" style="display: flex; flex-direction: column; gap: 12px;">
    <h1 style="font-size: 36px; line-height: 1.12; max-width: 360px;">{S.welcomeTitle}</h1>
    <p style="margin: 0; font-size: 15px; line-height: 1.45; color: #3F3C38;">{S.welcomeSub}</p>
  </div>
  <div data-m="cta" style="margin-top: 28px;">
    <button
      type="button"
      style="height: 46px; padding: 0 26px; border: 0; border-radius: 10px; background: #1D1C1A; color: #FFFFFF; font-size: 15px; font-weight: 600;"
      onclick={(e) => start(e.currentTarget)}><span class="lbl">{S.getStarted}</span></button>
    {#if error}<div class="err" role="alert" style="margin-top: 12px;" in:msgIn>{error}</div>{/if}
  </div>
  <div data-m="art" style="margin-top: auto; margin-left: -14px; margin-right: -14px;">
    <Art name="welcome" />
  </div>
</div>

<script lang="ts">
  import { onMount } from 'svelte';
  import BackBar from '../components/BackBar.svelte';
  import { failureText, isFailure } from '../lib/ipc';
  import { S } from '../lib/strings';
  import { store } from '../lib/store.svelte';

  let digits = $state('');
  let error = $state<string | null>(null);
  let busy = $state(false);
  let askLeave = $state(false);
  let input = $state<HTMLInputElement | null>(null);

  const box =
    'width: 52px; height: 64px; box-sizing: border-box; background: #FFFFFF; border-radius: 10px; display: flex; align-items: center; justify-content: center; font-size: 30px; font-weight: 600; font-variant-numeric: tabular-nums;';

  onMount(() => input?.focus());

  // The real text caret (the one the OS reports and magnifiers such as Wind follow) must sit
  // in the box that takes the next digit. The input is invisible and covers the row. Each
  // character advances exactly one box pitch (monospace, letter-spacing = pitch - 1ch), and
  // the left padding puts the caret after n characters in the middle of box n (after the 3rd
  // box the row has a 10 px split plus one more gap). After 6 digits the caret sits in the
  // right part of the last box, after its digit.
  const BOX = 52;
  const PITCH = 60; // box width + 8 px gap
  const SPLIT = 18; // the split span plus its extra gap
  function caretX(n: number): number {
    if (n >= 6) return 5 * PITCH + SPLIT + BOX - 6;
    return n * PITCH + (n >= 3 ? SPLIT : 0) + BOX / 2;
  }
  const caretPad = $derived(caretX(digits.length) - digits.length * PITCH);

  function onInput(e: Event) {
    const el = e.target as HTMLInputElement;
    digits = el.value.replace(/\D/g, '').slice(0, 6);
    el.value = digits;
    error = null;
    askLeave = false;
  }

  async function join(leaveOther: boolean) {
    busy = true;
    error = null;
    try {
      const r = await store.api!.call<{ name: string }>('pair.join', {
        code: digits,
        leave_other: leaveOther,
      });
      store.added = r.name;
      await store.refresh();
      store.go('connected', { replace: true });
    } catch (e) {
      if (isFailure(e) && e.code === 'in-group') askLeave = true;
      else error = failureText(e);
    } finally {
      busy = false;
    }
  }
</script>

<div style="flex: 1; display: flex; flex-direction: column; padding: 4px 24px 24px 24px; min-height: 0;">
  <BackBar onback={() => store.goBack('add')} />
  <div style="flex: 1; display: flex; flex-direction: column; justify-content: center; align-items: center; gap: 40px;">
    <div style="display: flex; flex-direction: column; gap: 8px; text-align: center;">
      <h1 style="font-size: 28px; line-height: 1.12;">{S.enterTitle}</h1>
      <p style="margin: 0; font-size: 15px; line-height: 1.45; color: #3F3C38;">{S.enterSub}</p>
    </div>
    <div style="position: relative;">
      <div role="group" aria-label={`Code, ${digits.length} of 6 digits entered`} style="display: flex; gap: 8px; align-items: center;">
        {#each [0, 1, 2, 3, 4, 5] as i}
          {#if i === 3}<span aria-hidden="true" style="width: 10px;"></span>{/if}
          {@const active = i === digits.length}
          <div data-testid="digit" style={`${box} border: ${active ? '2px solid #1D1C1A' : '1px solid #BDB8B1'};`}>
            {#if digits[i]}{digits[i]}{:else if active}<span style="display: block; width: 2px; height: 30px; background: #1D1C1A;"></span>{/if}
          </div>
        {/each}
      </div>
      <input
        bind:this={input}
        aria-label="Code"
        inputmode="numeric"
        autocomplete="off"
        maxlength="7"
        value={digits}
        oninput={onInput}
        onkeydown={(e) => e.key === 'Enter' && digits.length === 6 && !busy && join(false)}
        style={`position: absolute; inset: 0; width: 100%; height: 100%; box-sizing: border-box; margin: 0; padding: 0 0 0 ${caretPad}px; opacity: 0; border: 0; outline: none; background: transparent; cursor: text; font-family: ui-monospace, Menlo, Consolas, 'DejaVu Sans Mono', monospace; font-size: 30px; font-variant-numeric: tabular-nums; letter-spacing: calc(${PITCH}px - 1ch); text-indent: 0; text-align: left;`}
      />
    </div>
    {#if error}<div class="err" role="alert" style="max-width: 360px; margin-top: -22px;">{error}</div>{/if}
    {#if askLeave}
      <div role="alertdialog" aria-label={S.leaveOtherTitle} style="display: flex; flex-direction: column; gap: 10px; align-items: center; margin-top: -22px;">
        <div style="font-size: 14px; font-weight: 600; text-align: center;">{S.leaveOtherTitle}</div>
        <div style="display: flex; gap: 8px;">
          <button type="button" class="btn ghost" style="height: 38px; padding: 0 16px; font-size: 14px;" onclick={() => (askLeave = false)}>{S.cancel}</button>
          <button type="button" class="btn" style="height: 38px; padding: 0 16px; font-size: 14px; width: auto;" onclick={() => join(true)}>{S.leaveOtherYes}</button>
        </div>
      </div>
    {/if}
  </div>
  <div style="display: flex; flex-direction: column; align-items: center; gap: 16px;">
    <button type="button" class="btn" disabled={digits.length !== 6 || busy} onclick={() => join(false)}>
      {busy ? S.connecting : S.connect}
    </button>
  </div>
</div>

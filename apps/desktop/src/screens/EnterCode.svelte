<script lang="ts">
  import { onMount } from 'svelte';
  import BackBar from '../components/BackBar.svelte';
  import { failureText, isFailure } from '../lib/ipc';
  import { anim, canAnimate, IN, msgIn, OUT, reduced, shake } from '../lib/motion';
  import { S } from '../lib/strings';
  import { store } from '../lib/store.svelte';

  let digits = $state('');
  let error = $state<string | null>(null);
  /** Counts errors, so the same message rises in again. */
  let errorN = $state(0);
  let busy = $state(false);
  let askLeave = $state(false);
  let input = $state<HTMLInputElement | null>(null);
  let row = $state<HTMLElement | null>(null);
  /** E1: red outlines after a wrong code, until the boxes are clear. */
  let bad = $state(false);
  /** Boxes before this index are still red while the row clears from the right. */
  let redTo = $state(6);
  /** E1: the code no longer works; the boxes are grey until a new code is typed. */
  let dead = $state(false);
  let clearing = $state(false);

  const box =
    'width: 52px; height: 64px; box-sizing: border-box; background: #FFFFFF; border-radius: 10px; display: flex; align-items: center; justify-content: center; font-size: 30px; font-weight: 600; font-variant-numeric: tabular-nums; transition: border-color 220ms var(--ease-out), background-color 700ms var(--ease-in-out), color 700ms var(--ease-in-out);';

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
    let v = el.value.replace(/\D/g, '');
    // After a dead code, typing starts a new code: the old digits go.
    if (dead) v = v.length > digits.length ? v.slice(digits.length) : '';
    digits = v.slice(0, 6);
    el.value = digits;
    error = null;
    askLeave = false;
    bad = false;
    dead = false;
  }

  // E1 "Shake, then clear" (errors.html): one damped shake, red outlines, then the boxes
  // empty one by one from the right and the first box takes the caret again.
  function wrongCode() {
    bad = true;
    redTo = 6;
    shake(row);
    if (!canAnimate()) return clearNow();
    clearing = true;
    const r = reduced();
    const spans = row ? Array.from(row.querySelectorAll<HTMLElement>('[data-testid="digit"] > span.d')) : [];
    spans.forEach((s, i) => {
      const at = r ? 800 : 800 + (5 - i) * 90;
      anim(s, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: r ? 'none' : 'translateY(5px)' }], { duration: 200, delay: at, easing: IN, fill: 'forwards' });
      setTimeout(() => (redTo = Math.min(redTo, i)), at);
    });
    setTimeout(clearNow, r ? 1000 : 800 + 5 * 90 + 200);
  }
  function clearNow() {
    clearing = false;
    if (bad) {
      digits = '';
      if (input) input.value = '';
      bad = false;
      input?.focus();
    }
  }

  async function join(leaveOther: boolean) {
    if (clearing) return;
    busy = true;
    error = null;
    try {
      const r = await store.api!.call<{ name: string }>('pair.join', {
        code: digits,
        leave_other: leaveOther,
      });
      store.added = r.name;
      await store.refresh();
      store.go('connected', { replace: true, from: connectBtn });
    } catch (e) {
      if (isFailure(e) && e.code === 'in-group') askLeave = true;
      else {
        error = failureText(e);
        errorN += 1;
        const code = isFailure(e) ? e.code : '';
        if (code === 'wrong-code' || code === 'bad-code') wrongCode();
        // E1 dead code: the boxes fade grey in a slow wave (colour moves with a 70 ms lag each).
        else if (code === 'code-dead' || code === 'pair-failed') dead = true;
      }
    } finally {
      busy = false;
    }
  }
  let connectBtn = $state<HTMLElement | null>(null);

  function boxStyle(i: number): string {
    const active = i === digits.length && !dead && !clearing;
    const border = dead ? '1px solid #D9D6D1' : bad && i < redTo ? '1px solid #B42318' : active ? '2px solid #1D1C1A' : '1px solid #BDB8B1';
    const dim = dead ? 'background: rgba(255, 255, 255, 0); color: #6E6A64;' : 'color: #1D1C1A;';
    return `${box} border: ${border}; ${dim} transition-duration: ${bad && i < redTo ? 220 : 400}ms, 700ms, 700ms; transition-delay: 0ms, ${wave(i)}ms, ${wave(i)}ms;`;
  }

  /** E1 dead code: each box greys 70 ms after the one before it; reduced motion greys them together. */
  const wave = (i: number) => (dead && !reduced() ? i * 70 : 0);

  // E1: a typed digit settles in (scale 0.86 to 1).
  function digitIn(node: Element) {
    if (!canAnimate()) return { duration: 0 };
    anim(node, [{ opacity: 0, transform: reduced() ? 'none' : 'scale(0.86)' }, { opacity: 1, transform: 'none' }], { duration: 240, easing: OUT });
    return { duration: 240 };
  }
</script>

<div style="flex: 1; display: flex; flex-direction: column; padding: 4px 24px 24px 24px; min-height: 0;">
  <div data-m="back"><BackBar onback={() => store.goBack('add')} /></div>
  <div style="flex: 1; display: flex; flex-direction: column; justify-content: center; align-items: center; gap: 40px;">
    <div data-m="text" style="display: flex; flex-direction: column; gap: 8px; text-align: center;">
      <h1 style="font-size: 28px; line-height: 1.12;">{S.enterTitle}</h1>
      <p style="margin: 0; font-size: 15px; line-height: 1.45; color: #3F3C38;">{S.enterSub}</p>
    </div>
    <div data-m="digits" style="position: relative;">
      <div bind:this={row} role="group" aria-label={`Code, ${digits.length} of 6 digits entered`} style="display: flex; gap: 8px; align-items: center;">
        {#each [0, 1, 2, 3, 4, 5] as i}
          {#if i === 3}<span aria-hidden="true" style="width: 10px;"></span>{/if}
          {@const active = i === digits.length && !dead && !clearing}
          <div data-testid="digit" style={boxStyle(i)}>
            {#if digits[i]}{#key digits[i]}<span class="d" in:digitIn|global>{digits[i]}</span>{/key}{:else if active}<span style="display: block; width: 2px; height: 30px; background: #1D1C1A;"></span>{/if}
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
        onkeydown={(e) => e.key === 'Enter' && digits.length === 6 && !busy && !dead && !clearing && join(false)}
        style={`position: absolute; inset: 0; width: 100%; height: 100%; box-sizing: border-box; margin: 0; padding: 0 0 0 ${caretPad}px; opacity: 0; border: 0; outline: none; background: transparent; cursor: text; font-family: ui-monospace, Menlo, Consolas, 'DejaVu Sans Mono', monospace; font-size: 30px; font-variant-numeric: tabular-nums; letter-spacing: calc(${PITCH}px - 1ch); text-indent: 0; text-align: left;`}
      />
    </div>
    {#if error}
      {#key errorN}
        <div class="err" role="alert" style="max-width: 360px; margin-top: -22px;" in:msgIn|global={{ delay: 100 }}>{error}</div>
      {/key}
    {/if}
    {#if askLeave}
      <div role="alertdialog" aria-label={S.leaveOtherTitle} style="display: flex; flex-direction: column; gap: 10px; align-items: center; margin-top: -22px;" in:msgIn>
        <div style="font-size: 14px; font-weight: 600; text-align: center;">{S.leaveOtherTitle}</div>
        <div style="display: flex; gap: 8px; width: 320px;">
          <button type="button" class="btn ghost" style="height: 38px; padding: 0 16px; font-size: 14px; flex: 1; width: auto; white-space: nowrap;" onclick={() => (askLeave = false)}>{S.cancel}</button>
          <button type="button" class="btn" style="height: 38px; padding: 0 16px; font-size: 14px; flex: 1; width: auto; white-space: nowrap;" onclick={() => join(true)}>{S.leaveOtherYes}</button>
        </div>
      </div>
    {/if}
  </div>
  <div data-m="cta" style="display: flex; flex-direction: column; align-items: center; gap: 16px;">
    <button bind:this={connectBtn} type="button" class="btn" disabled={digits.length !== 6 || busy || dead} onclick={() => join(false)}>
      <span class="lbl">{busy ? S.connecting : S.connect}</span>
    </button>
  </div>
</div>

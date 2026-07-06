<script lang="ts">
  // Text section: size / bold-italic-underline / alignment / color (+ a text
  // label's background fill). ONE panel for single- and multi-selections (#135):
  // controls resolve via `sharedValue` and report a mixed state when the selected
  // objects disagree; writes fan out to the whole selection as one undo step.
  // Callers gate the section by the shared text-format capability.
  import type { EditorDoc, ObjectDoc } from '../doc.svelte';
  import Icon from '../Icon.svelte';
  import { alignValue, boolValue, colorValue, numberValue, sharedValue } from './values';
  import { writeStyleMany } from './persist-ops';

  let {
    doc,
    layoutId = '',
    objects,
  }: { doc: EditorDoc; layoutId?: string; objects: readonly Readonly<ObjectDoc>[] } = $props();

  let mFontSize = $derived(sharedValue(objects, (p) => numberValue(p.fontSize, 13)));
  let mBold = $derived(sharedValue(objects, (p) => boolValue(p.bold)));
  let mItalic = $derived(sharedValue(objects, (p) => boolValue(p.italic)));
  let mUnderline = $derived(sharedValue(objects, (p) => boolValue(p.underline)));
  let mAlign = $derived(sharedValue(objects, (p) => alignValue(p.align)));
  let mTextColor = $derived(sharedValue(objects, (p) => colorValue(p.textColor, '#1b1b1f')));
  let mTextBg = $derived(sharedValue(objects, (p) => colorValue(p.fill, '#ffffff')));

  // Background fill is a text-object attribute (Issue 7); shown only when every
  // selected object is a text label (the server's object_style() renders
  // `background:{fill}` for them).
  let allText = $derived(objects.length > 0 && objects.every((o) => o.kind === 'text'));

  function setStyle(key: string, value: string | number | boolean): void {
    void writeStyleMany(doc, layoutId, objects.map((o) => o.id), key, value);
  }
</script>

<section class="insp-sec">
  <span class="side-label">Text</span>
  <div class="insp-row">
    <span>Size</span>
    <input
      class="ctl-num"
      type="number"
      min="6"
      max="96"
      placeholder={mFontSize.mixed ? 'Mixed' : ''}
      value={mFontSize.mixed ? '' : mFontSize.value}
      onchange={(e) => setStyle('fontSize', Number(e.currentTarget.value))}
    />
  </div>
  <div class="seg-row">
    <div class="seg">
      <button
        type="button"
        class="seg-btn"
        class:active={!mBold.mixed && mBold.value}
        class:mixed={mBold.mixed}
        title="Bold"
        onclick={() => setStyle('bold', mBold.mixed ? true : !mBold.value)}
      ><b>B</b></button>
      <button
        type="button"
        class="seg-btn"
        class:active={!mItalic.mixed && mItalic.value}
        class:mixed={mItalic.mixed}
        title="Italic"
        onclick={() => setStyle('italic', mItalic.mixed ? true : !mItalic.value)}
      ><i>I</i></button>
      <button
        type="button"
        class="seg-btn"
        class:active={!mUnderline.mixed && mUnderline.value}
        class:mixed={mUnderline.mixed}
        title="Underline"
        onclick={() => setStyle('underline', mUnderline.mixed ? true : !mUnderline.value)}
      ><u>U</u></button>
    </div>
    <div class="seg">
      {#each ['left', 'center', 'right'] as a}
        <button
          type="button"
          class="seg-btn"
          class:active={!mAlign.mixed && mAlign.value === a}
          title={`Align ${a}`}
          onclick={() => setStyle('align', a)}
        ><Icon name={`align-${a}`} /></button>
      {/each}
    </div>
  </div>
  <div class="insp-row">
    <span>Color</span>
    <div class="insp-ctls">
      {#if mTextColor.mixed}<span class="mixed-tag">Mixed</span>{/if}
      <input
        class="swatch"
        type="color"
        value={mTextColor.value}
        onchange={(e) => setStyle('textColor', e.currentTarget.value)}
      />
    </div>
  </div>
  {#if allText}
    <div class="insp-row">
      <span>Background</span>
      <div class="insp-ctls">
        {#if mTextBg.mixed}<span class="mixed-tag">Mixed</span>{/if}
        <input
          class="swatch"
          type="color"
          value={mTextBg.value}
          onchange={(e) => setStyle('fill', e.currentTarget.value)}
        />
      </div>
    </div>
  {/if}
</section>

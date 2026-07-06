<script lang="ts">
  // Value-format section (#77 number/Boolean, #78 date/time) — contextual by the
  // bound field's kind (resolved through the binding, #79/#76). The controls
  // write the `format` sub-bag of the object's props; the server owns the actual
  // Browse/canvas render (crates/server/src/format.rs), and the panel's Sample
  // line asks that same formatter over `/design/format-sample`. Rendered only
  // while the selected field's kind has a value format at all.
  import type { EditorDoc } from '../doc.svelte';
  import { postJson } from '../../shared/http';
  import { llog, lerror } from '../log';
  import { colorValue } from './values';
  import { writeObjectProps } from './persist-ops';

  let {
    doc,
    layoutId = '',
    selectedId,
    props,
    fieldKind,
  }: {
    doc: EditorDoc;
    layoutId?: string;
    selectedId: number;
    props: Record<string, unknown>;
    fieldKind: string;
  } = $props();

  let isNumberFormat = $derived(fieldKind === 'number' || fieldKind === 'bool');
  let isDateFormat = $derived(fieldKind === 'date');
  let isTimeFormat = $derived(fieldKind === 'time');
  let isTimestampFormat = $derived(fieldKind === 'timestamp');

  let formatBag = $derived(asBag(props.format));
  // Date/Time write to the format bag directly for pure Date/Time fields, or into
  // the `date`/`time` sub-bags of a Timestamp's format (crates/server/src/format.rs).
  let dateBag = $derived(isTimestampFormat ? asBag(formatBag.date) : formatBag);
  let timeBag = $derived(isTimestampFormat ? asBag(formatBag.time) : formatBag);
  let dateComponents = $derived(
    Array.isArray(dateBag.components) ? (dateBag.components as Record<string, unknown>[]) : [],
  );

  let numberMode = $derived(bagStr(formatBag, 'mode', 'general'));
  let dateMode = $derived(bagStr(dateBag, 'mode', 'asEntered'));
  let timeMode = $derived(bagStr(timeBag, 'mode', 'asEntered'));
  let timeHas24 = $derived(bagBool(timeBag, 'hours24', true));
  let hasNegativeColor = $derived(typeof formatBag.negativeColor === 'string');

  // A representative raw value per kind so the Sample exercises every control.
  let sampleRaw = $derived(
    isNumberFormat
      ? '-1234.567'
      : isDateFormat
        ? '2003-12-25'
        : isTimeFormat
          ? '13:05:09'
          : isTimestampFormat
            ? '2003-12-25T13:05:09'
            : '',
  );
  // The Sample line is rendered by the SERVER's formatter (format.rs — the one
  // definition of the format rules; there is no client-side mirror). Debounced so
  // a burst of control edits costs one request; a stale response never overwrites
  // a newer one because the effect's cleanup cancels the pending timer and the
  // fetch result is dropped once superseded.
  let sample = $state<{ text: string; color: string | null }>({ text: '', color: null });
  let sampleRequest = 0;
  $effect(() => {
    const raw = sampleRaw;
    const kind = fieldKind;
    const format = formatBag;
    if (!kind) {
      sample = { text: '', color: null };
      return;
    }
    const id = ++sampleRequest;
    const timer = setTimeout(() => {
      postJson<{ text: string; color: string | null }>('/design/format-sample', { raw, kind, format })
        .then((f) => {
          if (id === sampleRequest) sample = f;
        })
        .catch((e) => lerror('persist', 'format sample failed', e));
    }, 120);
    return () => clearTimeout(timer);
  });

  function asBag(v: unknown): Record<string, unknown> {
    return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {};
  }
  function bagStr(b: Record<string, unknown>, key: string, fallback: string): string {
    return typeof b[key] === 'string' ? (b[key] as string) : fallback;
  }
  function bagBool(b: Record<string, unknown>, key: string, fallback: boolean): boolean {
    return typeof b[key] === 'boolean' ? (b[key] as boolean) : fallback;
  }
  function bagNum(b: Record<string, unknown>, key: string, fallback: number): number {
    return typeof b[key] === 'number' && Number.isFinite(b[key] as number) ? (b[key] as number) : fallback;
  }
  function defaultDateComponent(type: string): Record<string, unknown> {
    switch (type) {
      case 'dayOfWeek':
        return { type, style: 'long', leading: '' };
      case 'month':
        return { type, style: 'number', leadingZero: true, leading: '' };
      case 'day':
        return { type, leadingZero: false, leading: '' };
      case 'year':
        return { type, style: 'full', leading: '' };
      default:
        return { type };
    }
  }
  const DATE_COMPONENT_LABEL: Record<string, string> = {
    dayOfWeek: 'Day of week',
    month: 'Month',
    day: 'Day',
    year: 'Year',
  };

  // ── Value-format handlers ─────────────────────────────────────────────────
  // All writes merge into the object's `format` bag and persist through the same
  // doc-store + persistProps path as style/text edits, so they're undoable and the
  // server re-derives the object's style. `undefined` values are dropped by
  // JSON.stringify, which is how an optional key (e.g. negativeColor) is removed.

  async function commitFormat(next: Record<string, unknown>): Promise<void> {
    const merged = { ...props, format: next };
    llog('persist', 'inspector: set value format', { id: selectedId, format: next });
    await writeObjectProps(doc, layoutId, selectedId, merged, 'set value format');
  }
  // Number/Boolean write the format bag directly.
  function patchNumber(patch: Record<string, unknown>): void {
    void commitFormat({ ...formatBag, ...patch });
  }
  // Date/Time target the bag directly (pure field) or the timestamp sub-bag.
  function patchDate(patch: Record<string, unknown>): void {
    if (isTimestampFormat) void commitFormat({ ...formatBag, date: { ...dateBag, ...patch } });
    else void commitFormat({ ...formatBag, ...patch });
  }
  function patchTime(patch: Record<string, unknown>): void {
    if (isTimestampFormat) void commitFormat({ ...formatBag, time: { ...timeBag, ...patch } });
    else void commitFormat({ ...formatBag, ...patch });
  }
  function patchTimestamp(patch: Record<string, unknown>): void {
    void commitFormat({ ...formatBag, ...patch });
  }
  function setDateComponents(comps: Record<string, unknown>[]): void {
    patchDate({ components: comps });
  }
  function addDateComponent(type: string): void {
    setDateComponents([...dateComponents, defaultDateComponent(type)]);
  }
  function updateDateComponent(i: number, patch: Record<string, unknown>): void {
    setDateComponents(dateComponents.map((c, idx) => (idx === i ? { ...c, ...patch } : c)));
  }
  function removeDateComponent(i: number): void {
    setDateComponents(dateComponents.filter((_, idx) => idx !== i));
  }
  function moveDateComponent(i: number, up: boolean): void {
    const j = up ? i - 1 : i + 1;
    if (j < 0 || j >= dateComponents.length) return;
    const next = [...dateComponents];
    [next[i], next[j]] = [next[j], next[i]];
    setDateComponents(next);
  }
</script>

<section class="insp-sec">
  <span class="side-label">Value format</span>
  {#if isNumberFormat}
    {@render numberControls()}
  {/if}
  {#if isDateFormat}
    {@render dateControls()}
  {/if}
  {#if isTimeFormat}
    {@render timeControls()}
  {/if}
  {#if isTimestampFormat}
    <div class="fmt-sub">Date</div>
    {@render dateControls()}
    <div class="fmt-sub">Time</div>
    {@render timeControls()}
    <div class="insp-row">
      <span>Date/time gap</span>
      <input
        class="ctl-char"
        type="text"
        value={bagStr(formatBag, 'separator', ' ')}
        onchange={(e) => patchTimestamp({ separator: e.currentTarget.value })}
      />
    </div>
  {/if}
  <div class="fmt-sample">
    <span class="fmt-sample-label">Sample</span>
    <span class="fmt-sample-val" style={sample.color ? `color:${sample.color}` : ''}
      >{sample.text || '—'}</span
    >
  </div>
</section>

<!-- ── Value-format control snippets (#77/#78) ────────────────────────────────
     Reused for pure Number/Bool/Date/Time fields and, together, for a Timestamp
     (which formats a Date sub-bag + a Time sub-bag). The patch* helpers route each
     write to the right bag, so the same snippet serves both cases. -->
{#snippet numberControls()}
  <div class="insp-row">
    <span>Format</span>
    <select
      class="ctl-select ctl-select-auto"
      value={numberMode}
      onchange={(e) => patchNumber({ mode: e.currentTarget.value })}
    >
      <option value="general">General</option>
      <option value="asEntered">Leave as entered</option>
      <option value="boolean">Boolean</option>
      <option value="decimal">Decimal</option>
    </select>
  </div>
  {#if numberMode === 'boolean'}
    <div class="insp-row">
      <span>Non-zero as</span>
      <input
        class="ctl-input fmt-grow"
        type="text"
        placeholder="e.g. Yes"
        value={bagStr(formatBag, 'booleanNonZero', '')}
        onchange={(e) => patchNumber({ booleanNonZero: e.currentTarget.value })}
      />
    </div>
    <div class="insp-row">
      <span>Zero as</span>
      <input
        class="ctl-input fmt-grow"
        type="text"
        placeholder="e.g. No"
        value={bagStr(formatBag, 'booleanZero', '')}
        onchange={(e) => patchNumber({ booleanZero: e.currentTarget.value })}
      />
    </div>
  {:else if numberMode !== 'asEntered'}
    {#if numberMode === 'decimal'}
      <div class="insp-row">
        <span>Fixed decimals</span>
        <div class="insp-ctls">
          <input
            class="ctl-num"
            type="number"
            min="0"
            max="15"
            disabled={!bagBool(formatBag, 'fixedDecimals', false)}
            value={bagNum(formatBag, 'decimalDigits', 2)}
            onchange={(e) =>
              patchNumber({ decimalDigits: Math.min(Math.max(Math.round(Number(e.currentTarget.value) || 0), 0), 15) })}
          />
          <label class="toggle">
            <input
              type="checkbox"
              checked={bagBool(formatBag, 'fixedDecimals', false)}
              onchange={(e) => patchNumber({ fixedDecimals: e.currentTarget.checked })}
            />
            <span class="toggle-track"><span class="toggle-knob"></span></span>
          </label>
        </div>
      </div>
      <div class="insp-row">
        <span>Currency</span>
        <select
          class="ctl-select ctl-select-auto"
          value={bagStr(formatBag, 'currency', 'none')}
          onchange={(e) => patchNumber({ currency: e.currentTarget.value })}
        >
          <option value="none">None</option>
          <option value="leading">Leading</option>
          <option value="inside">Inside</option>
        </select>
      </div>
      {#if bagStr(formatBag, 'currency', 'none') !== 'none'}
        <div class="insp-row">
          <span>Symbol</span>
          <input
            class="ctl-char"
            type="text"
            placeholder="$"
            value={bagStr(formatBag, 'currencySymbol', '')}
            onchange={(e) => patchNumber({ currencySymbol: e.currentTarget.value })}
          />
        </div>
      {/if}
      <div class="insp-row">
        <span>Hide if zero</span>
        <label class="toggle">
          <input
            type="checkbox"
            checked={bagBool(formatBag, 'hideZero', false)}
            onchange={(e) => patchNumber({ hideZero: e.currentTarget.checked })}
          />
          <span class="toggle-track"><span class="toggle-knob"></span></span>
        </label>
      </div>
    {/if}
    <div class="insp-row">
      <span>Decimal separator</span>
      <input
        class="ctl-char"
        type="text"
        maxlength="1"
        value={bagStr(formatBag, 'decimalSeparator', '.')}
        onchange={(e) => patchNumber({ decimalSeparator: e.currentTarget.value || '.' })}
      />
    </div>
    <div class="insp-row">
      <span>Thousands separator</span>
      <select
        class="ctl-select ctl-select-auto"
        value={bagStr(formatBag, 'thousandsSeparator', '')}
        onchange={(e) => patchNumber({ thousandsSeparator: e.currentTarget.value })}
      >
        <option value="">None</option>
        <option value=",">Comma ,</option>
        <option value=".">Period .</option>
        <option value=" ">Space</option>
        <option value="'">Apostrophe '</option>
      </select>
    </div>
    <div class="insp-row">
      <span>Negatives</span>
      <select
        class="ctl-select ctl-select-auto"
        value={bagStr(formatBag, 'negativeStyle', 'minus')}
        onchange={(e) => patchNumber({ negativeStyle: e.currentTarget.value })}
      >
        <option value="minus">Minus −1234</option>
        <option value="parens">Parens (1234)</option>
      </select>
    </div>
    <div class="insp-row">
      <span>Negative color</span>
      <div class="insp-ctls">
        {#if hasNegativeColor}
          <input
            class="swatch"
            type="color"
            value={colorValue(formatBag.negativeColor, '#d70015')}
            onchange={(e) => patchNumber({ negativeColor: e.currentTarget.value })}
          />
        {/if}
        <label class="toggle">
          <input
            type="checkbox"
            checked={hasNegativeColor}
            onchange={(e) =>
              patchNumber({ negativeColor: e.currentTarget.checked ? colorValue(formatBag.negativeColor, '#d70015') : undefined })}
          />
          <span class="toggle-track"><span class="toggle-knob"></span></span>
        </label>
      </div>
    </div>
  {/if}
{/snippet}

{#snippet dateControls()}
  <div class="insp-row">
    <span>Date</span>
    <select
      class="ctl-select ctl-select-auto"
      value={dateMode}
      onchange={(e) => patchDate({ mode: e.currentTarget.value })}
    >
      <option value="asEntered">Leave as entered</option>
      <option value="predefined">Predefined</option>
      <option value="custom">Custom</option>
    </select>
  </div>
  {#if dateMode === 'predefined'}
    <div class="insp-row">
      <span>Style</span>
      <select
        class="ctl-select ctl-select-auto"
        value={bagStr(dateBag, 'predefined', 'mm/dd/yyyy')}
        onchange={(e) => patchDate({ predefined: e.currentTarget.value })}
      >
        <option value="mm/dd/yy">mm/dd/yy</option>
        <option value="mm/dd/yyyy">mm/dd/yyyy</option>
        <option value="dd/mm/yy">dd/mm/yy</option>
        <option value="dd/mm/yyyy">dd/mm/yyyy</option>
        <option value="yyyy-mm-dd">yyyy-mm-dd</option>
      </select>
    </div>
    <div class="insp-row">
      <span>Separator</span>
      <input
        class="ctl-char"
        type="text"
        maxlength="1"
        value={bagStr(dateBag, 'dateSeparator', bagStr(dateBag, 'predefined', 'mm/dd/yyyy').includes('-') ? '-' : '/')}
        onchange={(e) => patchDate({ dateSeparator: e.currentTarget.value })}
      />
    </div>
  {:else if dateMode === 'custom'}
    <div class="fmt-comps">
      {#each dateComponents as comp, i (i)}
        <div class="fmt-comp">
          <div class="fmt-comp-head">
            <span class="fmt-comp-name">{DATE_COMPONENT_LABEL[bagStr(comp, 'type', '')] ?? 'Part'}</span>
            <div class="insp-ctls">
              <button
                type="button"
                class="ord-btn"
                title="Move up"
                disabled={i === 0}
                onclick={() => moveDateComponent(i, true)}>↑</button
              >
              <button
                type="button"
                class="ord-btn"
                title="Move down"
                disabled={i === dateComponents.length - 1}
                onclick={() => moveDateComponent(i, false)}>↓</button
              >
              <button type="button" class="ord-btn" title="Remove" onclick={() => removeDateComponent(i)}>×</button>
            </div>
          </div>
          <div class="insp-row">
            <span>Leading</span>
            <input
              class="ctl-char"
              type="text"
              placeholder="sep"
              value={bagStr(comp, 'leading', '')}
              onchange={(e) => updateDateComponent(i, { leading: e.currentTarget.value })}
            />
          </div>
          {#if bagStr(comp, 'type', '') === 'dayOfWeek'}
            <div class="insp-row">
              <span>Style</span>
              <select
                class="ctl-select ctl-select-auto"
                value={bagStr(comp, 'style', 'long')}
                onchange={(e) => updateDateComponent(i, { style: e.currentTarget.value })}
              >
                <option value="long">Long</option>
                <option value="short">Short</option>
              </select>
            </div>
          {:else if bagStr(comp, 'type', '') === 'month'}
            <div class="insp-row">
              <span>Style</span>
              <select
                class="ctl-select ctl-select-auto"
                value={bagStr(comp, 'style', 'number')}
                onchange={(e) => updateDateComponent(i, { style: e.currentTarget.value })}
              >
                <option value="number">Number</option>
                <option value="short">Short</option>
                <option value="long">Long</option>
              </select>
            </div>
            {#if bagStr(comp, 'style', 'number') === 'number'}
              <div class="insp-row">
                <span>Leading zero</span>
                <label class="toggle">
                  <input
                    type="checkbox"
                    checked={bagBool(comp, 'leadingZero', true)}
                    onchange={(e) => updateDateComponent(i, { leadingZero: e.currentTarget.checked })}
                  />
                  <span class="toggle-track"><span class="toggle-knob"></span></span>
                </label>
              </div>
            {/if}
          {:else if bagStr(comp, 'type', '') === 'day'}
            <div class="insp-row">
              <span>Leading zero</span>
              <label class="toggle">
                <input
                  type="checkbox"
                  checked={bagBool(comp, 'leadingZero', false)}
                  onchange={(e) => updateDateComponent(i, { leadingZero: e.currentTarget.checked })}
                />
                <span class="toggle-track"><span class="toggle-knob"></span></span>
              </label>
            </div>
          {:else if bagStr(comp, 'type', '') === 'year'}
            <div class="insp-row">
              <span>Style</span>
              <select
                class="ctl-select ctl-select-auto"
                value={bagStr(comp, 'style', 'full')}
                onchange={(e) => updateDateComponent(i, { style: e.currentTarget.value })}
              >
                <option value="full">Full 2003</option>
                <option value="short">Short 03</option>
              </select>
            </div>
          {/if}
        </div>
      {/each}
      <div class="fmt-add">
        {#each ['dayOfWeek', 'month', 'day', 'year'] as t (t)}
          <button type="button" class="fmt-add-btn" onclick={() => addDateComponent(t)}>+ {DATE_COMPONENT_LABEL[t]}</button>
        {/each}
      </div>
    </div>
  {/if}
{/snippet}

{#snippet timeControls()}
  <div class="insp-row">
    <span>Time</span>
    <select
      class="ctl-select ctl-select-auto"
      value={timeMode}
      onchange={(e) => patchTime({ mode: e.currentTarget.value })}
    >
      <option value="asEntered">Leave as entered</option>
      <option value="predefined">Predefined</option>
      <option value="custom">Custom</option>
    </select>
  </div>
  {#if timeMode === 'predefined'}
    <div class="insp-row">
      <span>Style</span>
      <select
        class="ctl-select ctl-select-auto"
        value={bagStr(timeBag, 'predefined', 'hh:mm:ss')}
        onchange={(e) => patchTime({ predefined: e.currentTarget.value })}
      >
        <option value="hh:mm:ss">hh:mm:ss</option>
        <option value="hh:mm">hh:mm</option>
      </select>
    </div>
  {/if}
  {#if timeMode === 'custom'}
    <div class="insp-row">
      <span>Show seconds</span>
      <label class="toggle">
        <input
          type="checkbox"
          checked={bagBool(timeBag, 'showSeconds', true)}
          onchange={(e) => patchTime({ showSeconds: e.currentTarget.checked })}
        />
        <span class="toggle-track"><span class="toggle-knob"></span></span>
      </label>
    </div>
  {/if}
  {#if timeMode !== 'asEntered'}
    <div class="insp-row">
      <span>24-hour</span>
      <label class="toggle">
        <input
          type="checkbox"
          checked={timeHas24}
          onchange={(e) => patchTime({ hours24: e.currentTarget.checked })}
        />
        <span class="toggle-track"><span class="toggle-knob"></span></span>
      </label>
    </div>
    <div class="insp-row">
      <span>Separator</span>
      <input
        class="ctl-char"
        type="text"
        maxlength="1"
        value={bagStr(timeBag, 'timeSeparator', ':')}
        onchange={(e) => patchTime({ timeSeparator: e.currentTarget.value || ':' })}
      />
    </div>
    <div class="insp-row">
      <span>Hours leading zero</span>
      <label class="toggle">
        <input
          type="checkbox"
          checked={bagBool(timeBag, 'hoursLeadingZero', true)}
          onchange={(e) => patchTime({ hoursLeadingZero: e.currentTarget.checked })}
        />
        <span class="toggle-track"><span class="toggle-knob"></span></span>
      </label>
    </div>
    <div class="insp-row">
      <span>Min/sec leading zero</span>
      <label class="toggle">
        <input
          type="checkbox"
          checked={bagBool(timeBag, 'minutesSecondsLeadingZero', true)}
          onchange={(e) => patchTime({ minutesSecondsLeadingZero: e.currentTarget.checked })}
        />
        <span class="toggle-track"><span class="toggle-knob"></span></span>
      </label>
    </div>
    {#if !timeHas24}
      <div class="insp-row">
        <span>AM label</span>
        <input
          class="ctl-char"
          type="text"
          value={bagStr(timeBag, 'amLabel', 'AM')}
          onchange={(e) => patchTime({ amLabel: e.currentTarget.value })}
        />
      </div>
      <div class="insp-row">
        <span>PM label</span>
        <input
          class="ctl-char"
          type="text"
          value={bagStr(timeBag, 'pmLabel', 'PM')}
          onchange={(e) => patchTime({ pmLabel: e.currentTarget.value })}
        />
      </div>
      <div class="insp-row">
        <span>AM/PM placement</span>
        <select
          class="ctl-select ctl-select-auto"
          value={bagStr(timeBag, 'amPmPlacement', 'after')}
          onchange={(e) => patchTime({ amPmPlacement: e.currentTarget.value })}
        >
          <option value="after">After</option>
          <option value="before">Before</option>
          <option value="none">None</option>
        </select>
      </div>
    {/if}
  {/if}
{/snippet}

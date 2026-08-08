<script lang="ts">
  import type { CalendarDay } from '../types';

  // A year of daily counts as a contribution grid. The scale is quartile-based
  // rather than a fixed count, so a machine that runs 2 jobs a day and one that
  // runs 40 both produce a readable gradient instead of one all-pale grid and
  // one all-dark. The legend states the busiest day so the shading is anchored
  // to a real number rather than left as decoration.

  let { days = [], unit = 'runs' }: { days?: CalendarDay[]; unit?: string } = $props();

  const max = $derived(days.reduce((m, d) => Math.max(m, d.count), 0));
  /** 0 = empty, 1-4 = quartiles of the busiest day. */
  function level(n: number): number {
    if (n <= 0 || max <= 0) return 0;
    return Math.min(4, Math.ceil((n / max) * 4));
  }

  // Columns are weeks, rows are weekdays, so the grid reads left-to-right in
  // time. The first column is padded with blanks up to the year's first
  // weekday, otherwise every row would be off by a day.
  const weeks = $derived.by(() => {
    if (!days.length) return [] as (CalendarDay | null)[][];
    const out: (CalendarDay | null)[][] = [];
    let col: (CalendarDay | null)[] = [];
    const firstDow = new Date(`${days[0].date}T00:00:00`).getDay();
    for (let i = 0; i < firstDow; i++) col.push(null);
    for (const d of days) {
      col.push(d);
      if (col.length === 7) {
        out.push(col);
        col = [];
      }
    }
    if (col.length) {
      while (col.length < 7) col.push(null);
      out.push(col);
    }
    return out;
  });

  const total = $derived(days.reduce((n, d) => n + d.count, 0));
  const active = $derived(days.filter((d) => d.count > 0).length);

  // A month label sits above the first week that starts a new month.
  const months = $derived.by(() =>
    weeks.map((w, i) => {
      const first = w.find(Boolean);
      if (!first) return '';
      const d = new Date(`${first.date}T00:00:00`);
      const prev = weeks[i - 1]?.find(Boolean);
      const prevMonth = prev ? new Date(`${prev.date}T00:00:00`).getMonth() : -1;
      return d.getMonth() !== prevMonth ? d.toLocaleDateString(undefined, { month: 'short' }) : '';
    }),
  );

  const label = (d: CalendarDay) =>
    `${d.count} ${d.count === 1 ? unit.replace(/s$/, '') : unit} on ${new Date(`${d.date}T00:00:00`).toLocaleDateString(
      undefined,
      { month: 'short', day: 'numeric', year: 'numeric' },
    )}`;
</script>

<div class="cal">
  {#if !days.length}
    <p class="calempty">No history yet.</p>
  {:else}
    <div class="calscroll">
      <div class="calmonths" aria-hidden="true">
        {#each months as m, i (i)}<span class="calm">{m}</span>{/each}
      </div>
      <div class="calbody">
        <div class="caldow" aria-hidden="true"><span>Mon</span><span>Wed</span><span>Fri</span></div>
        <div class="calgrid" role="img" aria-label="{total} {unit} across {active} days in the last year">
          {#each weeks as w, i (i)}
            <div class="calweek">
              {#each w as d, k (k)}
                {#if d}
                  <span class="calcell l{level(d.count)}" title={label(d)}></span>
                {:else}
                  <span class="calcell blank"></span>
                {/if}
              {/each}
            </div>
          {/each}
        </div>
      </div>
    </div>
    <div class="calfoot">
      <span class="callegend">
        less
        <i class="calcell l0"></i><i class="calcell l1"></i><i class="calcell l2"></i><i class="calcell l3"></i><i
          class="calcell l4"
        ></i>
        more
      </span>
      <span class="caltot">
        <b>{total}</b>
        {unit} · busiest day <b>{max}</b>
      </span>
    </div>
  {/if}
</div>

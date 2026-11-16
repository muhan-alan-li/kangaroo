// One table, three stages.
//
// The dashboard's three lists differ only in which rows they hold. The columns, the two filter
// levels, the sorting, the paging and the measurement that decides how many rows fit are the
// same, so they live here once and each stage is a list of normalized rows plus the buttons its
// toolbar carries.
//
// The two filter levels, in the order they narrow:
//
//   1. the stage -- Open, Shortlisted, Applied. The tab bar is that filter, and the three lists
//      are disjoint, so this is a real partition rather than a highlight.
//   2. the location bucket, then the needle, inside the stage the reader is looking at.
//
// The page size is measured per table, from that table's own toolbar, head and pager. An earlier
// version measured the open table and reused the number for the others, which is why the
// shortlisted tab drew the wrong number of rows: same rows, different furniture above and below
// them.

import {useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState} from 'preact/hooks';
import type {ComponentChildren} from 'preact';
import {tierLabel} from './lib';

export type SortKey = 'date' | 'tier' | 'company' | 'title';
export type SizeChoice = 'fit' | '10' | '25' | '50';

export const SIZE_CHOICES = new Set<SizeChoice>(['fit', '10', '25', '50']);
export const FIT_MIN = 3;
export const FIT_MAX = 40;

// Fallbacks for the parts of the frame that are not measured yet on the first paint.
const ROW_FALLBACK_PX = 52;
const THEAD_FALLBACK_PX = 46;
const PAGER_FALLBACK_PX = 49;
const TOOLBAR_FALLBACK_PX = 60;
const PAGE_BOTTOM_PAD = 28;

// One row, as the table draws it. Each stage normalizes its own payload into this shape, so the
// markup never asks which list it came from.
export interface Row {
  id: string;
  date: string;          // the stage's own date: publish date, or the day it was applied to
  derived?: boolean;     // the date was derived from an age instead of published
  age?: string;          // "3d", beside the date
  tier?: string;
  company: string;
  title: string;
  location?: string;
  href?: string;         // the row can only be linked out; it cannot open the drawer
  clickable?: boolean;   // the row opens the detail drawer
  picked?: boolean;      // only meaningful when the stage passed onPick
  chip?: {text: string; cls: string; tip: string};
}

// ------------------------------------------------------------------ the two filter levels

// Level two, the needle: what the reader recognises a posting by, and its location. The stage
// has already been chosen by the time a row gets here.
export function filterRows(rows: Row[], needle: string, tier: string): Row[] {
  const q = needle.trim().toLowerCase();
  return rows.filter(r => {
    if (tier && r.tier !== tier) return false;
    if (!q) return true;
    return [r.company, r.title, r.location].join(' ').toLowerCase().includes(q);
  });
}

// The stage's date first (undated last, in either direction), then the location tier's rank,
// then plain text. One comparator, so the three tables sort identically.
export function sortRows(rows: Row[], key: SortKey, dir: number, tiers: string[]): Row[] {
  const out = rows.slice();
  out.sort((a, b) => {
    if (key === 'date'){
      const x = a.date || '', y = b.date || '';
      if (x && y) return x.localeCompare(y) * dir;
      if (!x && !y) return 0;
      return x ? -1 : 1;
    }
    if (key === 'tier'){
      const tx = tiers.indexOf(a.tier || ''), ty = tiers.indexOf(b.tier || '');
      return (tx - ty) * dir;
    }
    return ((a[key] || '').localeCompare(b[key] || '')) * dir;
  });
  return out;
}

// The buckets a stage can be filtered to, in tier order, with the seen-but-untiered ones behind.
// Counted over every row the stage holds rather than the rows now shown: picking a bucket must
// not empty the list of buckets you could pick next.
export function tierCounts(rows: Row[], order: string[]): {t: string; n: number}[] {
  const have: Record<string, number> = {};
  for (const r of rows) if (r.tier) have[r.tier] = (have[r.tier] || 0) + 1;
  const known = order.filter(t => have[t]);
  const extra = Object.keys(have).filter(t => !order.includes(t));
  return known.concat(extra).map(t => ({t, n: have[t]}));
}

// ------------------------------------------------------------------ how many rows fit

// The whole page, as one subtraction: the space below the table's own top, minus the furniture
// that is not rows. Pure and exported so the arithmetic can be checked without a browser.
export function rowsThatFit(m: {viewport: number; top: number; toolbar: number;
                                head: number; pager: number; row: number}): number {
  const avail = m.viewport - m.top - PAGE_BOTTOM_PAD - m.toolbar - m.head - m.pager;
  const n = Math.floor(avail / (m.row > 0 ? m.row : ROW_FALLBACK_PX));
  return Math.min(FIT_MAX, Math.max(FIT_MIN, Number.isFinite(n) ? n : FIT_MIN));
}

// ------------------------------------------------------------------ the table

export interface StageTableProps {
  id: string;
  hidden: boolean;
  label: string;                 // "open company or title", for the search box
  dateLabel: string;             // "Posted" or "Applied"
  rows: Row[];                   // the stage's rows, already filtered and sorted
  /** Every row the stage holds, for the location buckets and the empty state. */
  all: Row[];
  tiers: string[];
  needle: string;
  tier: string;
  onNeedle: (v: string) => void;
  onTier: (v: string) => void;
  sortKey: SortKey;
  sortDir: number;
  onSort: (k: SortKey) => void;
  size: SizeChoice;
  onSize: (v: SizeChoice) => void;
  info?: ComponentChildren;      // dim text at the right of the toolbar
  actions?: ComponentChildren;   // the stage's buttons
  empty: ComponentChildren;      // shown when the stage holds nothing at all
  onOpen?: (id: string) => void;
  onPick?: (id: string, on: boolean) => void;
}

export function StageTable(props: StageTableProps){
  const {id, hidden, rows, size} = props;
  const section = useRef<HTMLElement>(null);
  const wrap = useRef<HTMLDivElement>(null);
  const [page, setPage] = useState(1);
  const [fit, setFit] = useState(FIT_MIN);

  // Filtering or re-sorting is a new list, so it starts at the top. The key is the filters
  // themselves rather than the array, which is a new object on every render.
  const pageKey = `${props.needle}|${props.tier}|${props.sortKey}|${props.sortDir}`;
  useEffect(() => { setPage(1); }, [pageKey]);

  const pageSize = size === 'fit' ? fit : (parseInt(size, 10) || 10);
  const pages = Math.max(1, Math.ceil(rows.length / pageSize));
  const safePage = Math.min(Math.max(1, page), pages);
  const start = (safePage - 1) * pageSize;
  const pageRows = rows.slice(start, start + pageSize);

  // The measurement reads this table's own geometry -- its top, its toolbar, its head and its
  // pager -- so each stage gets its own answer. A hidden section has no height to measure, and
  // a null answer means "not measurable now", not "a page of zero rows".
  const measure = useCallback(() => {
    const sec = section.current;
    if (!sec || hidden) return;
    const head = sec.querySelector<HTMLElement>('thead');
    if (!head) return;
    const headH = head.getBoundingClientRect().height;
    if (!headH) return;
    const toolbar = sec.querySelector<HTMLElement>('.toolbar');
    const pager = sec.querySelector<HTMLElement>('.pager');
    const tr = sec.querySelector<HTMLElement>('tbody tr');
    setFit(rowsThatFit({
      viewport: window.innerHeight,
      top: sec.getBoundingClientRect().top,
      toolbar: toolbar?.offsetHeight || TOOLBAR_FALLBACK_PX,
      head: headH || THEAD_FALLBACK_PX,
      pager: pager?.offsetHeight || PAGER_FALLBACK_PX,
      row: tr ? tr.getBoundingClientRect().height : ROW_FALLBACK_PX,
    }));
  }, [hidden]);

  // Measure before the paint that would show a wrong number of rows, then again when the webfont
  // lands or the window is resized -- both change the row height under a fixed estimate.
  useLayoutEffect(() => {
    if (size === 'fit') measure();
  }, [size, measure, rows.length, props.sortKey, props.sortDir, hidden]);

  useEffect(() => {
    if (hidden || size !== 'fit') return;
    document.fonts?.ready.then(measure);
    let t: ReturnType<typeof setTimeout> | null = null;
    const onResize = () => {
      if (t) clearTimeout(t);
      t = setTimeout(measure, 150);
    };
    window.addEventListener('resize', onResize);
    return () => { if (t) clearTimeout(t); window.removeEventListener('resize', onResize); };
  }, [hidden, size, measure]);

  // After paint: trim a row at a time until the document truly fits, bounded so a tiny viewport
  // just scrolls a little. Only the visible table can overflow, so only it trims.
  useEffect(() => {
    if (hidden || size !== 'fit') return;
    if (document.documentElement.scrollHeight <= window.innerHeight + 1) return;
    if (fit <= FIT_MIN) return;
    const t = setTimeout(() => setFit(n => Math.max(FIT_MIN, n - 1)), 30);
    return () => clearTimeout(t);
  }, [hidden, size, fit, rows.length]);

  // Pin one page of space under the table so the pager never moves when a filter leaves fewer
  // rows than the page holds.
  useLayoutEffect(() => {
    const box = wrap.current;
    if (!box || hidden) return;
    const head = box.querySelector<HTMLElement>('thead');
    if (!head) return;
    const tr = box.querySelector<HTMLElement>('tbody tr');
    const rowH = tr ? tr.getBoundingClientRect().height : ROW_FALLBACK_PX;
    box.style.minHeight = Math.ceil(head.getBoundingClientRect().height + pageSize * rowH + 2) + 'px';
  }, [hidden, pageSize, rows.length, size]);

  const tiers = useMemo(() => tierCounts(props.all, props.tiers), [props.all, props.tiers]);

  const head = (key: SortKey, label: string) => (
    <th key={key} class={props.sortKey === key ? 'sorted' : ''} tabIndex={0}
        aria-sort={props.sortKey === key
          ? (props.sortDir > 0 ? 'ascending' : 'descending') : 'none'}
        data-dir={props.sortKey === key ? (props.sortDir > 0 ? 'asc' : 'desc') : undefined}
        onClick={() => props.onSort(key)}
        onKeyDown={e => {
          if (e.key === 'Enter' || e.key === ' '){
            e.preventDefault();
            props.onSort(key);
          }
        }}>{label}</th>
  );

  const hasChk = !!props.onPick;

  return (
    <section id={'stage-' + id} ref={section} role="tabpanel" hidden={hidden}>
      <div class="toolbar">
        <span class="prompt" aria-hidden="true">&gt;</span>
        <input type="search" id={'q-' + id} placeholder="Filter company or title…"
               aria-label={props.label} value={props.needle}
               onInput={e => props.onNeedle((e.target as HTMLInputElement).value)} />
        <select class="locfilter" aria-label="location" value={props.tier}
                onChange={e => props.onTier((e.target as HTMLSelectElement).value)}>
          <option value="">All locations</option>
          {tiers.map(({t, n}) => <option value={t} key={t}>{tierLabel(t)} ({n})</option>)}
        </select>
        <span class="pgspacer" />
        {props.info}
        {props.actions}
      </div>

      <div class="tablewrap" ref={wrap}>
        <table class={'shortlist' + (hasChk ? ' chkcol' : '')}>
          <thead><tr>
            {hasChk ? <th class="chk" /> : null}
            {head('date', props.dateLabel)}{head('tier', 'Location')}
            {head('company', 'Company')}{head('title', 'Title')}
          </tr></thead>
          <tbody id={'rows-' + id}>
            {pageRows.map(row => (
              <tr key={row.id} class={row.clickable ? 'clickable' : ''} data-id={row.id}
                  tabIndex={row.clickable ? 0 : undefined}
                  title={row.href && !row.clickable
                    ? 'no longer in the feed — the link opens the posting' : undefined}
                  onClick={row.clickable && props.onOpen
                    ? () => props.onOpen!(row.id) : undefined}
                  onKeyDown={row.clickable && props.onOpen ? e => {
                    if (e.key === 'Enter' || e.key === ' '){
                      e.preventDefault();
                      props.onOpen!(row.id);
                    }
                  } : undefined}>
                {/* The cell swallows the row's own handlers: ticking the box, or pressing
                    Space on the focused box, must select the row rather than open the drawer. */}
                {hasChk ? (
                  <td class="chk" onClick={e => e.stopPropagation()}
                      onKeyDown={e => e.stopPropagation()}>
                    <input type="checkbox" checked={!!row.picked}
                           aria-label={`select ${row.company} ${row.title || '(untitled)'}`}
                           onChange={e => props.onPick!(row.id,
                             (e.target as HTMLInputElement).checked)} />
                  </td>
                ) : null}
                <td class="posted">
                  {row.date
                    ? (row.derived
                      ? <span class="derived" title="derived from age">{row.date}</span>
                      : <span>{row.date}</span>)
                    : <span class="mut">—</span>}
                  {row.age ? <span class="age">{row.age}</span> : null}
                </td>
                <td>{row.tier
                  ? <span class={'tier tag tier-' + row.tier} title={row.location || ''}>
                      <i />{tierLabel(row.tier)}</span>
                  : <span class="mut">{row.location || '—'}</span>}</td>
                <td class="co" title={row.company}>{row.company}</td>
                <td class="title" title={row.title}>
                  {row.chip
                    ? <span class={'briefed' + row.chip.cls} title={row.chip.tip}>
                        {row.chip.text}</span>
                    : null}
                  {row.href
                    ? <a href={row.href} target="_blank" rel="noreferrer">{row.title} ↗</a>
                    : row.title}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {rows.length ? null : <div class="none">{props.all.length
          ? 'No rows match' : props.empty}</div>}
      </div>

      <div class="pager" id={'pager-' + id} style={{display: rows.length ? 'flex' : 'none'}}>
        <span class="pginfo">{rows.length
          ? `showing ${start + 1}\u2013${Math.min(start + pageSize, rows.length)} of ${rows.length}`
          : ''}</span>
        <span class="pgspacer" />
        <label class="pgsize">rows <select value={size}
            onChange={e => props.onSize((e.target as HTMLSelectElement).value as SizeChoice)}>
          <option value="fit">Fit</option><option value="10">10</option>
          <option value="25">25</option><option value="50">50</option>
        </select></label>
        <button class="pg" type="button" aria-label="previous page"
                disabled={safePage <= 1} onClick={() => setPage(safePage - 1)}>
          &lsaquo;</button>
        <span class="pgnum">{rows.length ? `page ${safePage} / ${pages}` : ''}</span>
        <button class="pg" type="button" aria-label="next page"
                disabled={safePage >= pages} onClick={() => setPage(safePage + 1)}>
          &rsaquo;</button>
      </div>
    </section>
  );
}

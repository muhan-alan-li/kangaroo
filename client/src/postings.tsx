// The postings page. One dashboard payload is fetched once, then everything is filtered,
// sorted and paged client-side so the table stays instant on a phone with no connection.

import {useCallback, useEffect, useLayoutEffect, useMemo, useState} from 'preact/hooks';
import {render} from 'preact';
import {fetchDashboard, removeApplied} from './api';
import type {AppliedRow, Dashboard, Posting} from './types';
import {initThemeToggle, postedOf, tierLabel, webUrl} from './lib';
import {Drawer} from './detail';

initThemeToggle();

// Newest posting first is the default: the freshest requisition is the one still worth a look.
// Anything undated sorts last in either direction (undated means "no publish date", not 1970).
const DEFAULT_SORT_KEY = 'posted';
const DEFAULT_SORT_DIR = -1;
const DEFAULT_PAGE_SIZE = 10;
const FIT_MIN = 3;
const FIT_MAX = 30;
const FRAME_PX = 72 + 20 + 28;   // topbar + page top pad + page bottom pad
const SIZE_CHOICES = new Set(['fit', '10', '25', '50']);

const storedSize = () => {
  try {
    const v = localStorage.getItem('jobhunt-pagesize') || 'fit';
    return SIZE_CHOICES.has(v) ? v : 'fit';
  } catch { return 'fit'; }
};

function App(){
  const [D, setD] = useState<Dashboard | null>(null);
  const [failed, setFailed] = useState(false);
  const [view, setView] = useState<'open' | 'applied'>('open');
  const [needle, setNeedle] = useState('');
  const [tier, setTier] = useState('');
  const [sortKey, setSortKey] = useState(DEFAULT_SORT_KEY);
  const [sortDir, setSortDir] = useState(DEFAULT_SORT_DIR);
  const [page, setPage] = useState(1);
  const [sizeChoice, setSizeChoice] = useState(() => storedSize());
  // The fit page size is measured: rows fill exactly the space between the table head and
  // the pager, so the whole page lands with no scrollbar.
  const [fitSize, setFitSize] = useState(FIT_MIN);
  const [picks, setPicks] = useState<string[]>([]);
  const [pid, setPid] = useState<string | null>(null);
  const [fitTick, setFitTick] = useState(0);   // bumped by fonts.ready and resize

  // Applied postings are not in the open list: marking one moves it to the applied section,
  // which is the whole point of the click.
  const appliedIds = useMemo(() =>
    new Set((D?.applied || []).map(a => String(a.id))), [D?.applied]);

  // Reload is the one path after a write whose response carried no applied list (an older
  // server, a proxy in the way): re-read the whole dashboard and repaint.
  const reload = useCallback(async () => {
    try {
      setD(await fetchDashboard());
      setFailed(false);
    } catch { /* keep what is on screen */ }
  }, []);

  useEffect(() => {
    fetchDashboard().then(D => setD(D), () => setFailed(true));
  }, []);

  // The fast path after a write: both applied routes answer with the whole applied list, so
  // marking or removing repaints from the response instead of re-fetching the dashboard.
  const setApplied = useCallback((rows?: AppliedRow[]) => {
    if (!Array.isArray(rows)) return void reload();
    setD(prev => prev ? {...prev, applied: rows} : prev);
  }, [reload]);

  const counts = useMemo(() => ({
    open: (D?.postings || []).filter(p => !appliedIds.has(String(p.id))).length,
    applied: (D?.applied || []).length,
  }), [D, appliedIds]);

  // Location buckets in tier order, with the seen-but-untiered ones carried behind.
  const locationOptions = useMemo(() => {
    if (!D) return [];
    const have: Record<string, number> = {};
    for (const p of D.postings){
      if (p.tier && !appliedIds.has(String(p.id)))
        have[p.tier] = (have[p.tier] || 0) + 1;
    }
    const known = D.tiers.filter(t => have[t]);
    const extra = Object.keys(have).filter(t => !D.tiers.includes(t));
    return known.concat(extra).map(t => ({t, n: have[t]}));
  }, [D, appliedIds]);

  // Filter, then sort. Applied rows drop out here rather than in the markup.
  const hits = useMemo(() => {
    if (!D) return [];
    const q = needle.toLowerCase();
    const out = D.postings.filter(p => {
      if (appliedIds.has(String(p.id))) return false;
      if (tier && p.tier !== tier) return false;
      if (!q) return true;
      const hay = [p.company, p.title, p.location, (p.keywords || []).join(' ')]
        .join(' ').toLowerCase();
      return hay.includes(q);
    });
    const dir = sortDir;
    out.sort((a, b) => {
      if (sortKey === 'posted'){
        const x = postedOf(a, D.ran_at).date, y = postedOf(b, D.ran_at).date;
        if (x && y) return x.localeCompare(y) * dir;
        if (!x && !y) return 0;
        return x ? -1 : 1;
      }
      if (sortKey === 'tier'){
        const tx = D.tiers.indexOf(a.tier || ''), ty = D.tiers.indexOf(b.tier || '');
        return (tx - ty) * dir;
      }
      const k = sortKey as 'company' | 'title';
      return ((a[k] || '').localeCompare(b[k] || '')) * dir;
    });
    return out;
  }, [D, appliedIds, needle, tier, sortKey, sortDir]);

  // Fit is measured, not configured. A manual rows choice disables the fit until "Fit" is
  // picked again; the choice persists in localStorage in chooseSize below.
  const pageSize = sizeChoice === 'fit'
    ? fitSize
    : (parseInt(sizeChoice, 10) || DEFAULT_PAGE_SIZE);

  const pages = Math.max(1, Math.ceil(hits.length / pageSize));
  const safePage = Math.min(Math.max(1, page), pages);
  const start = (safePage - 1) * pageSize;
  const pageRows = hits.slice(start, start + pageSize);

  // The ideal fit size is one arithmetic pass over the live layout; the trim loop then runs
  // below, after paint, because sub-pixel round-off only shows in the real scrollHeight.
  const measureIdeal = useCallback(() => {
    const viewbar = document.querySelector<HTMLElement>('.viewbar');
    const openview = document.getElementById('openview');
    const toolbar = openview?.querySelector<HTMLElement>('.toolbar');
    const thead = openview?.querySelector<HTMLElement>('.shortlist thead');
    const pager = document.getElementById('pager');
    const tr = openview?.querySelector<HTMLElement>('#rows tr');
    // The open pane is hidden on the applied view; its zero-height boxes would measure as a
    // huge page. A null answer means "not measurable now", not "a page of zero rows".
    if (!thead || !thead.getBoundingClientRect().height) return null;
    const rowH = tr ? tr.getBoundingClientRect().height : 52;
    return Math.min(FIT_MAX, Math.max(FIT_MIN, Math.floor(
      (window.innerHeight - FRAME_PX - (viewbar?.offsetHeight || 0)
       - (toolbar?.offsetHeight || 60) - thead.getBoundingClientRect().height
       - (pager?.offsetHeight || 49)) / rowH)));
  }, []);

  const fitNow = useCallback(() => {
    const ideal = view === 'open' ? measureIdeal() : null;
    if (ideal !== null) setFitSize(ideal);
    setFitTick(t => t + 1);
  }, [measureIdeal, view]);

  // Pin exactly one page of space under the table so the pager never moves under filtering.
  useLayoutEffect(() => {
    const openview = document.getElementById('openview');
    if (!openview || openview.hidden) return;
    const wrap = openview.querySelector<HTMLElement>('.tablewrap');
    const thead = openview.querySelector<HTMLElement>('.shortlist thead');
    const tr = openview.querySelector<HTMLElement>('#rows tr');
    if (wrap && thead){
      const rowH = tr ? tr.getBoundingClientRect().height : 52;
      wrap.style.minHeight = Math.ceil(thead.getBoundingClientRect().height + pageSize * rowH + 2) + 'px';
    }
  }, [pageSize, hits.length, sizeChoice, view]);

  // After paint: trim a row at a time until the document truly fits, bounded so a tiny
  // viewport just scrolls a little.
  useEffect(() => {
    if (sizeChoice !== 'fit') return;
    if (document.documentElement.scrollHeight <= window.innerHeight + 1) return;
    if (fitSize <= FIT_MIN) return;
    const t = setTimeout(() => setFitSize(s => s - 1), 30);
    return () => clearTimeout(t);
  }, [sizeChoice, fitSize, fitTick, hits.length, needle, tier]);

  // First fit once rows exist; re-fit when the webfonts land or the window is resized.
  useEffect(() => {
    if (sizeChoice === 'fit') fitNow();
  }, [view, hits.length > 0, sizeChoice, fitNow]);
  useEffect(() => {
    document.fonts?.ready.then(() => fitNow());
    let t: ReturnType<typeof setTimeout> | null = null;
    const onResize = () => {
      if (t) clearTimeout(t);
      t = setTimeout(fitNow, 150);
    };
    window.addEventListener('resize', onResize);
    return () => { if (t) clearTimeout(t); window.removeEventListener('resize', onResize); };
  }, [fitNow]);

  const chooseSize = (v: string) => {
    try { localStorage.setItem('jobhunt-pagesize', v); } catch { /* private mode */ }
    setSizeChoice(v);
    if (v !== 'fit'){
      const wrap = document.querySelector<HTMLElement>('.tablewrap');
      if (wrap) wrap.style.minHeight = '';
    }
  };

  function onSort(k: string){
    const dir = sortKey === k ? -sortDir : (k === 'posted' ? -1 : 1);
    setSortKey(k); setSortDir(dir); setPage(1);
  }

  // Drawer writes to the applied list; these callbacks hand its two responses to the paint.
  const drawerIsApplied = useCallback(
    (p: string) => (D?.applied || []).some(a => String(a.id) === String(p)), [D]);

  async function removeSelected(){
    if (!picks.length) return;
    try {
      const resp = await removeApplied(picks);
      setApplied(resp.applied);
      setPicks([]);
    } catch {
      setPicks([]);       // the reload shows whatever the server actually holds
      void reload();
    }
  }

  // The header outside the app root carries the last scan's date. This must stay above the
  // early returns: a hook cannot be skipped on the loading render.
  useEffect(() => {
    const topmeta = document.getElementById('topmeta');
    if (topmeta) topmeta.textContent = D?.ran_at ? D.ran_at.slice(0, 10) : '';
  }, [D]);

  if (failed){
    return <main class="page"><div class="card"><p class="bad">could not load</p></div></main>;
  }
  if (!D) return <main class="page"><div class="card"><p class="mut">loading…</p></div></main>;

  return <>
    <main class="page">
      <div class="card">
        <div class="viewbar">
          <div class="seg" id="seg" role="tablist" aria-label="posting views">
            <button type="button" role="tab" aria-selected={view === 'open'}
                    aria-controls="openview" class={view === 'open' ? 'on' : ''}
                    onClick={() => setView('open')}>Open <em>{counts.open}</em></button>
            <button type="button" role="tab" aria-selected={view === 'applied'}
                    aria-controls="appliedview" class={view === 'applied' ? 'on' : ''}
                    onClick={() => setView('applied')}>Applied <em>{counts.applied}</em></button>
          </div>
        </div>

        <section id="openview" role="tabpanel" hidden={view !== 'open'}>
          <div class="toolbar">
            <span class="prompt" aria-hidden="true">&gt;</span>
            <input type="search" id="q" placeholder="Filter company or title…"
                   aria-label="filter company or title" value={needle}
                   onInput={e => { setNeedle((e.target as HTMLInputElement).value); setPage(1); }} />
            <select id="locfilter" aria-label="location" value={tier}
                    onChange={e => { setTier((e.target as HTMLSelectElement).value); setPage(1); }}>
              <option value="">All locations</option>
              {locationOptions.map(({t, n}) =>
                <option value={t} key={t}>{tierLabel(t)} ({n})</option>)}
            </select>
          </div>

          <div class="tablewrap">
            <table class="shortlist"><thead><tr>
              {([['posted', 'Posted'], ['tier', 'Location'], ['company', 'Company'],
                 ['title', 'Title']] as const).map(([k, label]) =>
                <th key={k} class={sortKey === k ? 'sorted' : ''} tabIndex={0}
                    aria-sort={sortKey === k ? (sortDir > 0 ? 'ascending' : 'descending') : 'none'}
                    data-dir={sortKey === k ? (sortDir > 0 ? 'asc' : 'desc') : undefined}
                    onClick={() => onSort(k)}
                    onKeyDown={e => {
                      if (e.key === 'Enter' || e.key === ' '){
                        e.preventDefault();
                        onSort(k);
                      }
                    }}>{label}</th>)}
            </tr></thead>
            <tbody id="rows">
              {pageRows.map(p => <OpenRow p={p} ranAt={D.ran_at} key={p.id} open={setPid} />)}
            </tbody></table>
            {hits.length ? null : <div class="none" id="empty">No rows match</div>}
          </div>

          <div class="pager" id="pager" style={{display: hits.length ? 'flex' : 'none'}}>
            <span class="pginfo" id="pginfo">
              {hits.length ? `showing ${start + 1}\u2013${Math.min(start + pageSize, hits.length)} of ${hits.length}` : ''}
            </span>
            <span class="pgspacer" />
            <label class="pgsize">rows <select id="pgsize" value={sizeChoice}
                onChange={e => chooseSize((e.target as HTMLSelectElement).value)}>
              <option value="fit">Fit</option><option value="10">10</option>
              <option value="25">25</option><option value="50">50</option>
            </select></label>
            <button class="pg" id="pgprev" type="button" aria-label="previous page"
                    disabled={safePage <= 1} onClick={() => setPage(safePage - 1)}>&lsaquo;</button>
            <span class="pgnum" id="pgnum">
              {hits.length ? `page ${safePage} / ${pages}` : ''}</span>
            <button class="pg" id="pgnext" type="button" aria-label="next page"
                    disabled={safePage >= pages} onClick={() => setPage(safePage + 1)}>&rsaquo;</button>
          </div>
        </section>

        <section id="appliedview" role="tabpanel" hidden={view !== 'applied'}>
          <div class="toolbar appliedbar">
            <label class="selall">
              <input type="checkbox" id="selall"
                     checked={picks.length > 0 && picks.length === (D.applied || []).length}
                     indeterminate={picks.length > 0 && picks.length < (D.applied || []).length}
                     onChange={e => {
                       const on = (e.target as HTMLInputElement).checked;
                       setPicks(on ? (D.applied || []).map(a => a.id) : []);
                     }} />
              Select all
            </label>
            <span class="pgspacer" />
            <button class="btn" id="removeSel" type="button" disabled={!picks.length}
                    onClick={removeSelected}>
              {picks.length ? `Remove ${picks.length} selected` : 'Remove selected'}
            </button>
          </div>
          <div class="tablewrap">
            <table class="shortlist applied"><thead><tr>
              <th class="chk" /><th>Applied</th><th>Company</th><th>Title</th><th>Location</th>
            </tr></thead>
            <tbody id="appliedrows">
              {(D.applied || []).map(a => (
                <tr key={a.id}>
                  <td class="chk">
                    <input type="checkbox" class="aplpick"
                           checked={picks.includes(a.id)}
                           aria-label={`select ${a.company || ''} ${a.title || '(untitled)'}`}
                           onChange={e => {
                             const on = (e.target as HTMLInputElement).checked;
                             setPicks(on ? picks.concat(a.id)
                                         : picks.filter(i => i !== a.id));
                           }} />
                  </td>
                  <td class="posted">{a.applied_at
                    ? a.applied_at.slice(0, 10) : <span class="mut">—</span>}</td>
                  <td class="co">{a.company || ''}</td>
                  <td class="title">{webUrl(a.url)
                    ? <a href={webUrl(a.url)} target="_blank" rel="noreferrer">
                        {a.title || '(untitled)'} ↗</a>
                    : (a.title || '(untitled)')}</td>
                  <td>{a.tier
                    ? <span class={'tier tag tier-' + a.tier} title={a.location || ''}>
                        <i />{tierLabel(a.tier)}</span>
                    : <span class="mut">{a.location || '—'}</span>}</td>
                </tr>
              ))}
            </tbody></table>
            {(D.applied || []).length ? null :
              <div class="none" id="appliedempty">Nothing applied to yet. Open a posting and
                click its apply link — it moves here.</div>}
          </div>
        </section>
      </div>
    </main>

    <div id="scrim" class={pid ? 'on' : ''} aria-hidden="true" onClick={() => setPid(null)} />
    <Drawer pid={pid} ranAt={D.ran_at} isApplied={drawerIsApplied} setApplied={setApplied}
            close={() => setPid(null)} />
  </>;
}

// One open-list row. Reuses the classes of the retired markup so styles.css is untouched.
function OpenRow(props: {p: Posting; ranAt?: string; open: (pid: string) => void}){
  const p = props.p;
  const age = (p.age_days === null || p.age_days === undefined) ? '' : String(p.age_days) + 'd';
  const po = postedOf(p, props.ranAt);
  const tier = p.tier || '';
  return (
    <tr class="clickable" data-id={p.id} data-tier={tier} tabIndex={0}
        onClick={() => props.open(p.id)}
        onKeyDown={e => {
          if (e.key === 'Enter' || e.key === ' '){
            e.preventDefault();
            props.open(p.id);
          }
        }}>
      <td class="posted">
        {po.date ? (po.derived
          ? <span class="derived" title="derived from age">{po.date}</span>
          : <span>{po.date}</span>)
        : <span class="mut">—</span>}
        {age ? <span class="age">{age}</span> : null}
        {p.expires_in_days != null && p.expires_in_days <= 2
          ? <span class="closing">{p.expires_in_days}d left</span> : null}
      </td>
      <td><span class={'tier tag tier-' + tier} title={p.location || ''}>
        <i />{tierLabel(tier)}</span></td>
      <td class="co" title={p.company}>{p.company}</td>
      <td class="title" title={p.title}>{p.title}</td>
    </tr>
  );
}

render(<App />, document.getElementById('root') || document.body);

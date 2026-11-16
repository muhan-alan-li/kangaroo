// The postings page: one payload, three stages of one table.
//
// Everything downstream of the fetch is client-side, so the table stays instant on a phone with
// no connection. The lists themselves -- columns, filters, sorting, paging and the measurement
// that decides how many rows fit -- live in `table.tsx`, because all three stages draw the same
// table. What is left here is the pipeline:
//
//   Open  -- every posting the feed carries that has no brief and no application
//   Shortlisted -- every posting with a brief that has not been applied to
//   Applied -- the applied snapshots, which outlive the feed
//
// Stage is the first filter level and the tab bar is its control; the location bucket and the
// needle are the second, applied inside whichever stage is showing.

import {useCallback, useEffect, useMemo, useRef, useState} from 'preact/hooks';
import {render} from 'preact';
import {fetchDashboard, removeApplied, startBriefs} from './api';
import type {AppliedRow, Dashboard, Posting} from './types';
import {initThemeToggle, postedOf, webUrl} from './lib';
import {Drawer} from './detail';
import {SIZE_CHOICES, StageTable, filterRows, sortRows} from './table';
import type {Row, SizeChoice, SortKey} from './table';

initThemeToggle();

const DEFAULT_SORT_KEY: SortKey = 'date';
const DEFAULT_SORT_DIR = -1;
// Above this a batch confirm is worth the interruption: every id is a model call.
const BATCH_CONFIRM_AT = 20;

type Stage = 'open' | 'shortlist' | 'applied';

// One row a batch skipped or could not start, named from the posting it refers to: the server
// answers ids, the reader needs a company and a title.
interface BriefRow {id: string; company: string; title: string; message?: string}

// What the batch dialog reports. Only rows that did not start are carried, because those are
// the ones the reader has to be told about.
interface BatchReport {
  total: number;
  started: number;
  skipped: BriefRow[];
  failed: BriefRow[];
}

const storedSize = (): SizeChoice => {
  try {
    const v = (localStorage.getItem('jobhunt-pagesize') || 'fit') as SizeChoice;
    return SIZE_CHOICES.has(v) ? v : 'fit';
  } catch { return 'fit'; }
};

// What an open row says about a brief it already has. The pipeline now moves a briefed row to
// Shortlisted, so this only appears for a retryable failure or for a row another tab is briefing.
function briefChip(p: Posting){
  if (p.brief_status === 'ok')
    return {text: 'brief', cls: '', tip: 'a brief already exists — Generate briefs skips this row'};
  if (p.brief_status === 'running')
    return {text: 'briefing…', cls: '',
            tip: 'a brief is generating now — Generate briefs skips this row'};
  if (p.brief_status === 'failed')
    return {text: 'brief failed', cls: ' err', tip: 'the last brief failed — Generate briefs tries again'};
  return undefined;
}

function App(){
  const [D, setD] = useState<Dashboard | null>(null);
  const [failed, setFailed] = useState(false);
  const [stage, setStage] = useState<Stage>('open');
  // The second filter level, shared by the three stages: it is one table, so switching stages
  // keeps the bucket and the needle the reader set.
  const [needle, setNeedle] = useState('');
  const [tier, setTier] = useState('');
  const [sortKey, setSortKey] = useState(DEFAULT_SORT_KEY);
  const [sortDir, setSortDir] = useState(DEFAULT_SORT_DIR);
  const [size, setSize] = useState<SizeChoice>(() => storedSize());
  // Two selections, one per stage that has one: Open's drives brief generation, Applied's
  // drives removal, and they must not carry across the stage switch.
  const [openPicks, setOpenPicks] = useState<string[]>([]);
  const [appliedPicks, setAppliedPicks] = useState<string[]>([]);
  const [batchBusy, setBatchBusy] = useState(false);
  const [batchNote, setBatchNote] = useState('');
  const [batchReport, setBatchReport] = useState<BatchReport | null>(null);
  const [pid, setPid] = useState<string | null>(null);

  // Applied postings are not in any other list: marking one moves it here, which is the point.
  const appliedIds = useMemo(() =>
    new Set((D?.applied || []).map(a => String(a.id))), [D?.applied]);
  // A picked id is a bare string and the row that explains it is in the payload, so a batch
  // result is named by looking the row up here rather than asking the server to repeat it.
  const byId = useMemo(() =>
    new Map((D?.postings || []).map(p => [String(p.id), p] as const)), [D?.postings]);

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

  // ---------------------------------------------------------------- the three stages' rows

  // A posting that has a brief is not open any more -- the server already drops it, and the
  // applied filter here covers the instant between the drawer's Apply and the next reload.
  const openRows = useMemo<Row[]>(() => (D?.postings || [])
    .filter(p => !appliedIds.has(String(p.id)))
    .map(p => {
      const po = postedOf(p, D?.ran_at);
      return {
        id: String(p.id),
        date: po.date,
        derived: po.derived,
        age: p.age_days === null || p.age_days === undefined ? undefined : p.age_days + 'd',
        tier: p.tier,
        company: p.company,
        title: p.title,
        location: p.location,
        clickable: true,
        picked: openPicks.includes(String(p.id)),
        chip: briefChip(p),
      };
    }), [D, appliedIds, openPicks]);

  // A live row opens the drawer like an open row; a retired one can only be linked out, because
  // /api/posting/<id> has nothing left to show for a posting the feed has let go. Its company
  // and title come from the brief, so the row still says what it is.
  const shortlistRows = useMemo<Row[]>(() => (D?.shortlisted || [])
    .filter(r => !appliedIds.has(String(r.id)))
    .map(r => {
      const po = postedOf(r, D?.ran_at);
      return {
        id: String(r.id),
        date: po.date,
        derived: po.derived,
        age: r.age_days === null || r.age_days === undefined ? undefined : r.age_days + 'd',
        tier: r.tier,
        company: r.company,
        title: r.title || '(untitled)',
        location: r.location,
        clickable: r.live,
        href: r.live ? undefined : webUrl(r.url) || undefined,
      };
    }), [D, appliedIds]);

  const appliedRows = useMemo<Row[]>(() => (D?.applied || []).map(a => ({
    id: String(a.id),
    date: a.applied_at ? a.applied_at.slice(0, 10) : '',
    tier: a.tier,
    company: a.company,
    title: a.title || '(untitled)',
    location: a.location,
    href: webUrl(a.url) || undefined,
    picked: appliedPicks.includes(String(a.id)),
  })), [D, appliedPicks]);

  // Filter level two, then the sort. The stage's own date is the default order.
  const visible = useCallback((rows: Row[]) =>
    sortRows(filterRows(rows, needle, tier), sortKey, sortDir, D?.tiers || []),
  [needle, tier, sortKey, sortDir, D?.tiers]);

  const openHits = useMemo(() => visible(openRows), [visible, openRows]);
  const shortlistHits = useMemo(() => visible(shortlistRows), [visible, shortlistRows]);
  const appliedHits = useMemo(() => visible(appliedRows), [visible, appliedRows]);

  const counts = {
    open: openRows.length, shortlist: shortlistRows.length, applied: appliedRows.length,
  };

  // A selection only means anything while its rows are on screen. Filtering the table, or
  // marking one of the picked rows applied, must not leave hidden ids behind for the batch to
  // spend a model call on.
  useEffect(() => {
    setOpenPicks(prev => {
      if (!prev.length) return prev;
      const live = new Set(openHits.map(r => r.id));
      const next = prev.filter(id => live.has(id));
      return next.length === prev.length ? prev : next;
    });
  }, [openHits]);

  // How many of the selection the batch will skip: a brief that already exists, or a run still
  // in flight. Shown before the button is pressed, so the dialog afterwards confirms what the
  // toolbar already said rather than announcing it.
  const briefedPicks = useMemo(() => {
    let n = 0;
    for (const id of openPicks){
      const status = byId.get(id)?.brief_status;
      if (status === 'ok' || status === 'running') n++;
    }
    return n;
  }, [openPicks, byId]);

  // ------------------------------------------------------------------------- the writes

  const drawerIsApplied = useCallback(
    (p: string) => (D?.applied || []).some(a => String(a.id) === String(p)), [D]);

  async function removeSelected(){
    if (!appliedPicks.length) return;
    try {
      const resp = await removeApplied(appliedPicks);
      setApplied(resp.applied);
      setAppliedPicks([]);
    } catch {
      setAppliedPicks([]);   // the reload shows whatever the server actually holds
      void reload();
    }
  }

  const chooseSize = (v: SizeChoice) => {
    try { localStorage.setItem('jobhunt-pagesize', v); } catch { /* private mode */ }
    setSize(v);
  };

  function onSort(k: SortKey){
    const dir = sortKey === k ? -sortDir : (k === 'date' ? -1 : 1);
    setSortKey(k); setSortDir(dir);
  }

  // Batch generation: one request launches every selected listing that has no brief yet and
  // answers with what each id did. A skip is the rule, not an error -- a select-all must not
  // spend a model call on a row the reader has already read -- but a button that quietly does
  // nothing for six of eight rows reads as broken, so the skipped rows are named in a dialog.
  // The dashboard is re-read at the end to pick up the runs that just started.
  async function generateSelected(){
    const ids = openPicks;
    if (!ids.length || batchBusy) return;
    const willRun = ids.length - briefedPicks;
    if (willRun > BATCH_CONFIRM_AT && !window.confirm(
        `Generate briefs for ${willRun} listing${willRun === 1 ? '' : 's'}? Each one is a model call.`
        + (briefedPicks ? ` ${briefedPicks} already ha${briefedPicks === 1 ? 's' : 've'}`
                          + ' a brief and will be skipped.' : ''))) return;
    setBatchBusy(true);
    setBatchNote('');
    setBatchReport(null);
    // Named before the request, because the reload at the end may retire the row.
    const named = (id: string): BriefRow => {
      const p = byId.get(id);
      return {id, company: p?.company || '', title: p?.title || 'this listing'};
    };
    try {
      const resp = await startBriefs(ids);
      const parts = [`started ${resp.started || 0}`];
      if (resp.skipped) parts.push(`${resp.skipped} skipped, brief already exists`);
      if (resp.failed) parts.push(`${resp.failed} failed`);
      setBatchNote(parts.join(' · '));
      const withMessage = (state: string) => (resp.results || [])
        .filter(r => r.state === state)
        .map(r => ({...named(r.id), message: r.message || state}));
      const skipped = withMessage('skipped');
      const failures = withMessage('failed');
      if (skipped.length || failures.length){
        setBatchReport({total: ids.length, started: resp.started || 0, skipped,
                        failed: failures});
      }
      setOpenPicks([]);
    } catch (err){
      setBatchNote('could not start: ' + ((err as Error).message || 'request failed'));
    } finally {
      setBatchBusy(false);
      void reload();
    }
  }

  // A batch leaves runs in flight, and the shortlisted stage is where they land, so the
  // dashboard is re-read every 10s while anything is running -- bounded, like the drawer's own
  // poll, so a marker that never clears cannot poll forever.
  const runningBriefs = useMemo(() =>
    (D?.shortlisted || []).filter(r => r.status === 'running').length, [D]);
  useEffect(() => {
    if (!runningBriefs) return;
    let ticks = 0;
    const timer = setInterval(() => {
      if (++ticks > 60){ clearInterval(timer); return; }
      void reload();
    }, 10000);
    return () => clearInterval(timer);
  }, [runningBriefs, reload]);

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

  const selAll = (rows: Row[], picks: string[], set: (v: string[]) => void) => {
    const picked = rows.filter(r => picks.includes(r.id)).length;
    const all = rows.length > 0 && picked === rows.length;
    return (
      <label class="selall">
        <input type="checkbox" checked={all}
               indeterminate={picked > 0 && !all}
               onChange={e => set((e.target as HTMLInputElement).checked
                 ? rows.map(r => r.id) : [])} />
        Select all
      </label>
    );
  };

  const shared = {
    needle, tier, onNeedle: setNeedle, onTier: setTier,
    sortKey, sortDir, onSort, size, onSize: chooseSize, tiers: D.tiers,
  };

  return <>
    <main class="page">
      <div class="card">
        <div class="viewbar">
          <div class="seg" id="seg" role="tablist" aria-label="posting stages">
            {([['open', 'Open'], ['shortlist', 'Shortlisted'], ['applied', 'Applied']] as const)
              .map(([id, label]) => (
                <button key={id} type="button" role="tab" aria-selected={stage === id}
                        aria-controls={'stage-' + id} class={stage === id ? 'on' : ''}
                        onClick={() => setStage(id)}>
                  {label} <em>{counts[id]}</em>
                </button>
              ))}
          </div>
        </div>

        <StageTable id="open" label="filter open company or title" dateLabel="Posted"
                    hidden={stage !== 'open'} rows={openHits} all={openRows} {...shared}
                    onOpen={setPid} onPick={(id, on) => setOpenPicks(
                      on ? openPicks.concat(id) : openPicks.filter(i => i !== id))}
                    actions={<>
                      {selAll(openHits, openPicks, setOpenPicks)}
                      <button class="btn" type="button"
                              disabled={!openPicks.length || batchBusy}
                              onClick={generateSelected}>
                        {batchBusy ? 'Starting…'
                          : openPicks.length ? `Generate briefs (${openPicks.length})`
                          : 'Generate briefs'}
                      </button>
                      {briefedPicks ? (
                        <span class="batchhint" role="status"
                              title="these already have a brief, so the button skips them">
                          {briefedPicks} of {openPicks.length} already briefed
                        </span>
                      ) : null}
                      {batchNote ? <span class="batchnote" role="status">{batchNote}</span> : null}
                    </>}
                    empty="Nothing open. Every posting the feed carries has a brief or is
                           already on the applied list." />

        <StageTable id="shortlist" label="filter shortlisted company or title"
                    dateLabel="Posted" hidden={stage !== 'shortlist'}
                    rows={shortlistHits} all={shortlistRows} {...shared} onOpen={setPid}
                    info={<span class="pginfo">
                      {shortlistRows.length ? `${shortlistRows.length} with a brief` : ''}
                    </span>}
                    empty="No briefs yet. Select listings in Open and press Generate briefs —
                           each one lands here, and the applied stage takes it from there when
                           you apply." />

        <StageTable id="applied" label="filter applied company or title" dateLabel="Applied"
                    hidden={stage !== 'applied'} rows={appliedHits} all={appliedRows}
                    {...shared}
                    onPick={(id, on) => setAppliedPicks(
                      on ? appliedPicks.concat(id) : appliedPicks.filter(i => i !== id))}
                    actions={<>
                      {selAll(appliedHits, appliedPicks, setAppliedPicks)}
                      <button class="btn" type="button" disabled={!appliedPicks.length}
                              onClick={removeSelected}>
                        {appliedPicks.length ? `Remove ${appliedPicks.length} selected`
                          : 'Remove selected'}
                      </button>
                    </>}
                    empty="Nothing applied to yet. Open a posting and click its apply link — it
                           moves here." />
      </div>
    </main>

    {batchReport ? (
      <BatchDialog report={batchReport}
                   close={() => setBatchReport(null)}
                   shortlist={() => { setBatchReport(null); setStage('shortlist'); }} />
    ) : null}

    <div id="scrim" class={pid ? 'on' : ''} aria-hidden="true" onClick={() => setPid(null)} />
    <Drawer pid={pid} ranAt={D.ran_at} isApplied={drawerIsApplied} setApplied={setApplied}
            close={() => setPid(null)} />
  </>;
}

// The batch dialog. It exists for one case: the reader picked a selection and some of it was
// skipped because those briefs are already generated. That is correct behaviour, so the tone
// is information rather than error, and the dialog offers the one place the existing briefs
// can be read instead of leaving the reader to work out why the count came back short.
function BatchDialog(props: {report: BatchReport; close: () => void; shortlist: () => void}){
  const {report} = props;
  const closeBtn = useRef<HTMLButtonElement>(null);
  // Focus lands on Close, not on the Jump button: Escape and Enter must both dismiss, and a
  // reflex Enter must never navigate away from the table.
  useEffect(() => { closeBtn.current?.focus(); }, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') props.close(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [props.close]);

  const many = (n: number) => n === 1 ? '' : 's';
  const heading = report.started
    ? `Started ${report.started} of ${report.total} brief${many(report.total)}`
    : 'No briefs started';
  const lede = [
    report.skipped.length
      ? `${report.skipped.length} already ha${report.skipped.length === 1 ? 's' : 've'} a brief,`
        + ' so no model call was spent on ' + (report.skipped.length === 1 ? 'it' : 'them') + '.'
      : '',
    report.failed.length ? `${report.failed.length} could not be started.` : '',
  ].filter(Boolean).join(' ');

  const rows = (items: BriefRow[]) => (
    <ul class="modallist">
      {items.map(r => (
        <li key={r.id}>
          <span class="mco" title={r.company}>{r.company || '—'}</span>
          <span class="mti" title={r.title}>{r.title}</span>
          <span class="mmsg">{r.message}</span>
        </li>
      ))}
    </ul>
  );

  return (
    <div class="modalwrap" onClick={props.close}>
      <div class="modal" role="dialog" aria-modal="true" aria-labelledby="batchdlgtitle"
           onClick={e => e.stopPropagation()}>
        <h3 id="batchdlgtitle">{heading}</h3>
        {lede ? <p class="modalsub">{lede}</p> : null}
        {report.skipped.length ? (
          <section class="modalblock">
            <h4>Already has a brief</h4>
            {rows(report.skipped)}
          </section>
        ) : null}
        {report.failed.length ? (
          <section class="modalblock">
            <h4>Could not start</h4>
            {rows(report.failed)}
          </section>
        ) : null}
        <div class="modalact">
          {report.skipped.length ? (
            <button class="btn" type="button" onClick={props.shortlist}>
              Show them in Shortlisted
            </button>
          ) : null}
          <button class="btn ghost" type="button" ref={closeBtn} onClick={props.close}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

render(<App />, document.getElementById('root') || document.body);

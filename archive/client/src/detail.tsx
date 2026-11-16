// The detail drawer. Clicking a row opens the posting: its own facts, the cached posting text,
// and the brief. Nothing is generated until the button is pressed, because generating one is a
// model call (and, in the agent engine, a web research run).

import {Fragment} from 'preact';
import {useEffect, useRef, useState} from 'preact/hooks';
import {fetchPosting, markApplied, removeApplied, triggerBrief, triggerContacts} from './api';
import type {AppliedRow, Brief, Detail, Posting, Reword} from './types';
import {money, postedOf, tierLabel, webUrl} from './lib';
import {BriefFooter, ContactCard, KeywordTable, RewordBlock} from './detailParts';

// Poll every 7s and stop when the server's `running` flag drops. The server marks a run stale
// after 15 minutes, so this cannot spin forever even if the generator dies.
const POLL_MS = 7000;
const POLL_MAX_TICKS = 100;

interface DrawerProps {
  pid: string | null;
  ranAt?: string;
  isApplied: (pid: string) => boolean;
  setApplied: (rows?: AppliedRow[]) => void;
  close: () => void;
}

export function Drawer(props: DrawerProps){
  const open = !!props.pid;
  const [d, setD] = useState<Detail | null>(null);
  const [tab, setTab] = useState<'summary' | 'resume' | 'contact'>('summary');
  const [busy, setBusy] = useState(false);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);
  const pidRef = useRef<string | null>(null);
  const drawerRef = useRef<HTMLElement | null>(null);

  const stopPoll = () => {
    if (timer.current){ clearInterval(timer.current); timer.current = null; }
  };

  async function load(pid: string): Promise<Detail | null> {
    try {
      const fresh = await fetchPosting(pid);
      if (pidRef.current !== pid) return null;    // the user moved to another posting
      setD(fresh);
      return fresh;
    } catch (err){
      if (pidRef.current === pid) setD({error: String((err as Error).message)});
      return null;
    }
  }

  function startBrief(pid: string){
    setBusy(true);
    triggerBrief(pid).catch(() => { /* the next read shows whatever happened */ }).then(() => {
      if (pidRef.current !== pid) return;
      // Keep the button in its "starting" state until the first read comes back; otherwise a
      // fast click can start a second run in the gap between the POST and the state refresh.
      void load(pid).then(loaded => {
        setBusy(false);
        if (loaded?.running) watch(pid);
      });
    });
  }

  // Second level: the Hermes agent contact search. Same polling contract as the brief.
  function startContacts(pid: string){
    setBusy(true);
    triggerContacts(pid).catch(() => { /* the same */ }).then(() => {
      if (pidRef.current !== pid) return;
      void load(pid).then(() => {
        setBusy(false);
        watch(pid);
      });
    });
  }

  function watch(pid: string){
    if (pidRef.current !== pid) return;   // another posting replaced this run
    stopPoll();
    let ticks = 0;
    timer.current = setInterval(async () => {
      ticks++;
      if (pidRef.current !== pid) return stopPoll();
      const loaded = await load(pid);
      if ((loaded && !loaded.running) || ticks > POLL_MAX_TICKS) stopPoll();
    }, POLL_MS);
  }

  // Clicks delegated through the drawer body, so the panes can be re-rendered freely. The
  // action links are anchors, so they are matched BEFORE the plain-anchor pass-through below.
  function onBodyClick(e: MouseEvent){
    const el = e.target as HTMLElement;
    const undo = el.closest('[data-unapply]') as HTMLElement | null;
    if (undo?.dataset.unapply){
      e.preventDefault();
      removeApplied([undo.dataset.unapply]).then(
        resp => props.setApplied(resp?.applied),
        () => props.setApplied(undefined),   // reload: the server is the truth
      );
      return;
    }
    const regen = el.closest('[data-regen]') as HTMLElement | null;
    if (regen?.dataset.regen){
      e.preventDefault();
      startBrief(regen.dataset.regen);
      return;
    }
    const contacts = el.closest('[data-contacts]') as HTMLElement | null;
    if (contacts?.dataset.contacts){ startContacts(contacts.dataset.contacts); return; }
    // Posting links and mailto pass through to the browser.
  }

  // Apply opens the posting synchronously, in the click, because a window.open after an await
  // is pop-up blocked; the mark lands in the background.
  function handleApply(p: Posting){
    const url = webUrl(p.url);
    if (url) window.open(url, '_blank', 'noopener,noreferrer');
    markApplied(p.id).then(
      resp => props.setApplied(resp?.applied),
      () => {},
    );
  }

  // Load when the drawer is opened for a posting; clear the poll on close and unmount.
  useEffect(() => {
    stopPoll();
    pidRef.current = props.pid;
    setD(null);
    setTab('summary');
    setBusy(false);
    if (props.pid){
      drawerRef.current?.scrollTo({top: 0});
      void load(props.pid);
    }
  }, [props.pid]);
  useEffect(() => stopPoll, []);

  // Escape closes the drawer, as it did before the rewrite.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') props.close(); };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [open, props.close]);

  const p = d?.posting;
  const b = d?.brief || {};
  const title = d?.error ? d.error : p ? (p.title || '(untitled)') : 'loading…';
  const po = p ? postedOf(p, props.ranAt) : {date: '', derived: false};

  return (
    <aside id="drawer" ref={drawerRef} class={open ? 'on' : ''} aria-hidden={!open}
           aria-label="posting detail" inert={!open}>
      <div class="dh">
        <button class="x" type="button" title="close (Esc)" aria-label="close"
                onClick={props.close}>&times;</button>
        <h3 id="dtitle">{title}</h3>
        <div class="dsub" id="dsub">
          {!d || d.error ? '' :
            <>
              {(p?.company || '')}&nbsp;·&nbsp;{(p ? tierLabel(p.tier) : '')}&nbsp;·&nbsp;
              score {p ? String(p.score ?? '—') : '—'}
              {po.date ? <> &nbsp;·&nbsp; {po.derived ? '~' : ''}{po.date}</> : null}
            </>}
        </div>
        <div class={'dtabs' + (d && !d.error ? ' on' : '')} id="dtabs"
             role="tablist" aria-label="posting detail">
          {(['summary', 'resume', 'contact'] as const).map(t =>
            <button type="button" key={t} data-tab={t} class={tab === t ? 'on' : ''}
                    role="tab" aria-selected={tab === t}
                    onClick={() => setTab(t)}>
              {t === 'resume' ? 'Résumé' : t.charAt(0).toUpperCase() + t.slice(1)}
            </button>)}
        </div>
      </div>
      <div id="dbody" class="dbody" onClick={onBodyClick}>
        <div class={'dpane' + (tab === 'summary' ? ' on' : '')} data-pane="summary"
             role="tabpanel">
          <SummaryPane d={d} busy={busy} isApplied={props.isApplied} onApply={handleApply} onBrief={startBrief} />
          <BriefFooter b={b} />
        </div>
        <div class={'dpane' + (tab === 'resume' ? ' on' : '')} data-pane="resume"
             role="tabpanel">
          <ResumePane d={d} />
          <BriefFooter b={b} />
        </div>
        <div class={'dpane' + (tab === 'contact' ? ' on' : '')} data-pane="contact"
             role="tabpanel">
          <ContactPane d={d} busy={busy} onDeep={startContacts} />
          <BriefFooter b={b} />
        </div>
      </div>
    </aside>
  );
}

function SummaryPane(props: {
  d: Detail | null;
  busy: boolean;
  isApplied: (pid: string) => boolean;
  onApply: (p: Posting) => void;
  onBrief: (pid: string) => void;
}){
  const {d, busy} = props;
  if (!d) return <section><p class="spin">loading…</p></section>;
  if (d.error) return <section><p class="bad">could not load: {d.error}</p></section>;
  const p = d.posting || ({} as Posting);
  const b = d.brief || {};
  const ok = b.status === 'ok';

  const facts: [string, string][] = [
    ['Location', p.location || ''],
    ['Source', (p.source || '') + (p.ats ? ' / ' + p.ats : '')],
    ['Comp', money(p)],
    ['ID', p.id || ''],
  ];
  if (p.level_ii) facts.push(['Level', 'II']);
  if (p.requires_us_work_auth) facts.push(['US work auth', 'Required']);
  if (p.below_comp_floor) facts.push(['Comp floor', 'Below']);

  const total = d.description_length || 0;
  const cap = total > 12000 ? ', trimmed' : '';
  const linked = webUrl(p.url);
  const applied = props.isApplied(p.id);
  return <>
    <section>
      <div class="kv">
        {facts.map(([k, v]) => (
          <Fragment key={k}>
            <div class="k2">{k}</div>
            <div class="v2">{v}</div>
          </Fragment>
        ))}
      </div>
      {(linked || applied) ? <div class="actbar">
        {applied
          ? <>
              <span class="applied-badge">Applied</span>
              {linked
                ? <a class="btn ghost" href={linked} target="_blank" rel="noreferrer">Open posting ↗</a>
                : null}
              <button class="linkbtn" type="button" data-unapply={p.id}>Undo</button>
            </>
          : <>
              <button class="btn apply" type="button" onClick={() => props.onApply(p)}>Apply ↗</button>
              <span class="applyhint">Opens the posting and moves it to Applied.</span>
            </>}
      </div> : null}
    </section>
    {ok
      ? <>
          {b.verdict ? <section><p class="verdict">{b.verdict}</p></section> : null}
          {(b.key_skills || []).length
            ? <section><h4>What decides it</h4>
                <ul class="tight keypoints">
                  {(b.key_skills || []).map((x, i) => <li key={i}>{x}</li>)}
                </ul>
              </section>
            : null}
        </>
      : <section>
          <BriefMissing d={d} b={b} busy={busy} onBrief={() => props.onBrief(p.id)} />
        </section>}
    {d.description ? <section class="posttext">
      <details>
        <summary>Posting text ({total.toLocaleString()} chars{cap})</summary>
        <div class="body">{d.description}</div>
      </details>
    </section> : null}
  </>;
}

// What the Summary pane shows when no brief exists yet. The copy and the one action share a
// bordered empty state, so the button cannot float loose under the previous paragraph.
function BriefMissing(props: {
  d: Detail; b: Brief; busy: boolean; onBrief: () => void;
}){
  const {d, b, busy, onBrief} = props;
  if (d.running || busy) return (
    <div class="briefempty running">
      <span class="spinner" aria-hidden="true" />
      <div class="be-copy">
        <h4>Generating the brief…</h4>
        <p class="mut">The first read can take a minute.</p>
      </div>
    </div>
  );
  return (
    <div class="briefempty">
      <div class="be-copy">
        <h4>{b.status === 'failed' ? 'Brief generation failed' : 'Fit brief'}</h4>
        {b.status === 'failed'
          ? <p class="bad">The last run failed{b.error ? ': ' + b.error : ''}.</p>
          : d.stale ? <p class="bad">The previous run stopped without finishing.</p> : null}
        <p class="mut">No brief yet. Generate one for résumé-tailoring advice and an
          honest fit verdict.</p>
      </div>
      {d.has_script
        ? <button class="btn" type="button" onClick={onBrief}>
            {b.status === 'failed' ? 'Retry brief' : 'Generate brief'}
          </button>
        : <p class="bad be-note">Brief generator unavailable.</p>}
    </div>
  );
}

function ResumePane({d}: {d: Detail | null}){
  if (!d) return <section><p class="spin">loading…</p></section>;
  const b = d.brief || {};
  if (b.status !== 'ok') return (
    <section><p class="mut">{d.running ? 'Generating the brief…'
      : 'No brief yet — generate one from the Summary tab to get résumé-tailoring advice.'}</p></section>
  );
  const t = b.tailoring || {};
  const reword: (Reword | string)[] = t.reword || [];
  return <>
    {t.lead_with ? <section><h4>Lead with</h4><p>{t.lead_with}</p></section> : null}
    {reword.length ? <section><h4>Rewrite</h4><RewordBlock items={reword} /></section> : null}
    {(t.missing || []).length
      ? <section class="dont"><h4>Do not claim</h4>
          <ul class="tight">{(t.missing || []).map((x, i) => <li key={i}>{x}</li>)}</ul>
        </section>
      : null}
    {t.notes ? <section><h4>Notes</h4><p>{t.notes}</p></section> : null}
    <section><h4>Terms in the posting</h4><KeywordTable p={d.posting || ({} as Posting)} /></section>
  </>;
}

function ContactPane({d, busy, onDeep}: {
  d: Detail | null; busy: boolean; onDeep: (pid: string) => void;
}){
  if (!d) return <section><p class="spin">loading…</p></section>;
  const p = d.posting || ({} as Posting);
  const b = d.brief || {};
  const ok = b.status === 'ok';
  const running = !!d.running;
  const known = [...(p.emails || []), ...(p.verified_emails || [])];
  const contacts = b.contacts || [];
  const shown = contacts.length + known.length;
  return <>
    {running ? <section><p class="spin">Searching the web…</p></section> : null}
    {known.length
      ? <section><h4>Addresses in the posting</h4>
          {known.map(a => <div class="addr"><a href={'mailto:' + a}>{a}</a></div>)}
        </section>
      : null}
    {contacts.length
      ? <section>
          <h4>Researched contacts</h4>
          {b.contacts_source ? <p class="ksub">via {b.contacts_source}</p> : null}
          {contacts.map((c, i) => <ContactCard c={c} key={i} />)}
        </section>
      : null}
    {!shown && !running
      ? <section><p class="mut">{ok
          ? 'No contact was found for this posting.'
          : 'No brief yet — generate one from the Summary tab, or run the deep search below.'}</p></section>
      : null}
    {b.contact_notes ? <section>
      <details><summary>Contact search notes</summary><p>{b.contact_notes}</p></details>
    </section> : null}
    {!running && d.has_script ? <section class="deepsec">
      <h4>Deep search</h4>
      <p class="mut">The default brief covers résumé tailoring only. The Hermes agent can
        take a separate pass over the public web — team pages, profiles, and public commit
        metadata.</p>
      <p><button class="btn ghost" type="button" data-contacts={p.id} disabled={busy}>
        {busy ? 'starting…' : 'Run Hermes contact search'}
      </button></p>
    </section> : null}
  </>;
}

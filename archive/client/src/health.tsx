// Entry point for the scan-health page. Same payload as the postings page; only the
// scan/board sections are rendered here. Deliberately neutral — a board that didn't answer
// is routine (usually a transient 403/429), not an alarm.

import {useEffect, useState} from 'preact/hooks';
import {render} from 'preact';
import {fetchDashboard} from './api';
import type {Dashboard} from './types';
import {initThemeToggle} from './lib';

initThemeToggle();

const num = (n: number | undefined) => (n === null || n === undefined) ? '—' : Number(n).toLocaleString();

function App(){
  const [D, setD] = useState<Dashboard | null>(null);
  const [err, setErr] = useState('');
  useEffect(() => { fetchDashboard().then(setD, e => setErr(String((e as Error).message))); }, []);
  useEffect(() => {
    const topmeta = document.getElementById('topmeta');
    if (topmeta && D) topmeta.textContent = D.ran_at ? D.ran_at.slice(0, 10) : '';
  }, [D]);

  if (err) return <main class="page"><div class="card"><p class="bad">could not load: {err}</p></div></main>;
  if (!D) return <main class="page"><div class="card"><p class="mut">loading…</p></div></main>;

  const ran = D.ran_at ? D.ran_at.replace('T', ' ').slice(0, 16) : '—';
  const failing = D.boards_failing || 0;
  const kinds = [...new Set((D.failing || []).map(b => b.error_kind).filter(Boolean))];
  const next = (D.failing || []).map(b => b.next_check_at || '').filter(Boolean).sort()[0] || '';
  let failnote = 'Every board answered the last sweep.';
  if (failing){
    failnote = `${failing} board${failing === 1 ? '' : 's'} didn't answer`
      + (kinds.length ? ` (${kinds.join(', ')})` : '')
      + (next ? ` · next retry ${next.slice(0, 16).replace('T', ' ')}` : '') + '.';
  }
  const cost = Object.entries(D.source_seconds || {}).sort((a, b) => b[1] - a[1]);
  const sweep = D.sweep || {};

  return <>
    <main class="page">
      <div class="card hero">
        <div class="heroin" id="hero">
          <div>
            <div class="herohead">{Number(D.boards_total || 0).toLocaleString()} boards checked</div>
            <div class="herosub">last scan {ran} · {D.window}d window · sweep {sweep.verdict || '—'}</div>
          </div>
        </div>
        <div class="statgrid" id="statgrid">
          {([['Answering', num(D.boards_ok) + ' / ' + num(D.boards_total)],
             ["Didn't answer", num(D.boards_failing)],
             ['Live postings', num(D.matched)],
             ['Collected', num(D.collected)]]) .map(([label, value]) =>
            <div class="stat" key={label}>
              <div class="n">{value}</div><div class="l">{label}</div>
            </div>)}
        </div>
      </div>

      <div class="card">
        <div class="cardhead"><h2>Boards &amp; scan</h2></div>
        <div class="kvblock" id="scan">
          <div class="k2">Ran</div><div class="v2">{(D.ran_at || '—').replace('T', ' ').slice(0, 19)}</div>
          <div class="k2">Boards checked</div><div class="v2">{num(D.boards_total)}</div>
          <div class="k2">Answering</div><div class="v2">{num(D.boards_ok)}</div>
          <div class="k2">Live postings</div><div class="v2">{num(D.matched)}</div>
          <div class="k2">Swept</div><div class="v2">{sweep.count != null ? sweep.count : '—'}</div>
          <div class="k2">Sweep result</div><div class="v2">{sweep.verdict || '—'}</div>
          <div class="k2">Source time</div><div class="v2">{D.source_total ?? '—'}s</div>
        </div>
        <p class="sumnote" id="failnote">{failnote}</p>
        <details class="inset">
          <summary>Boards that didn't answer (<span id="failmeta">{failing ? failing : 'none'}</span>)</summary>
          <table class="data"><thead><tr>
            <th>Board</th><th class="r">Fails</th><th>Kind</th><th>Last error</th><th>Next check</th>
          </tr></thead>
          <tbody id="failrows">
            {(D.failing || []).map((b, i) => (
              <tr key={i}>
                <td>{b.name}</td>
                <td class="r">{b.consecutive_failures}</td>
                <td class="dim">{b.error_kind}</td>
                <td class="dim">{b.last_error}</td>
                <td class="r">{(b.next_check_at || '').slice(0, 16).replace('T', ' ')}</td>
              </tr>
            ))}
          </tbody></table>
          {(D.failing || []).length ? null :
            <div class="none">Nothing here — every board answered.</div>}
        </details>
        <details class="inset">
          <summary>Source cost</summary>
          <table class="data"><thead><tr>
            <th>Source</th><th class="r">Collected</th><th class="r">Seconds</th></tr>
          </thead>
          <tbody id="costrows">
            {cost.length ? null : <tr><td colSpan={3} class="dim">No timing recorded</td></tr>}
            {cost.map(([k, v]) => (
              <tr key={k}>
                <td>{k}</td><td class="r">{((D.sources || {})[k] ?? 0)}</td><td class="r">{v.toFixed(1)}</td>
              </tr>
            ))}
            {cost.length
              ? <tr class="total">
                  <td>Total</td><td class="r">{(D.collected || 0).toLocaleString()}</td>
                  <td class="r">{D.source_total ?? '—'}</td>
                </tr>
              : null}
          </tbody></table>
        </details>
      </div>
    </main>
  </>;
}

render(<App />, document.getElementById('root') || document.body);

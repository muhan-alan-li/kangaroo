// Pure view functions: every one takes data and returns markup. No fetching, no state.
import {esc, postedOf} from './util.js?v=20261026a';

// The canonical location buckets. The scan's `tier` already aggregates every spelling of a
// place -- Burnaby and Richmond into vancouver, "USA - Remote"
// and "US - Remote" into remote_us -- so this is a rename of the ranking field, not a second
// classifier, and it needs no model call.
const LOCATION_LABEL = {
  vancouver: 'Vancouver', remote_canada: 'Remote Canada', toronto: 'Toronto',
  seattle: 'Seattle', remote_us: 'Remote US',
  remote_unspecified: 'Remote',
};

export function renderLocations(D){
  const have = {};
  D.postings.forEach(p => { const t = p.tier || ''; if (t) have[t] = (have[t] || 0) + 1; });
  const order = D.tiers.concat(Object.keys(have).filter(t => !D.tiers.includes(t)));
  document.getElementById('locfilter').innerHTML = '<option value="">All locations</option>'
    + order.filter(t => have[t]).map(t =>
        `<option value="${esc(t)}">${esc(LOCATION_LABEL[t] || t)} (${have[t]})</option>`).join('');
}

export function renderRows(D){
  const rowsHtml = D.postings.map((p) => {
    const age = (p.age_days === null || p.age_days === undefined) ? '' : p.age_days + 'd';
    const po = postedOf(p, D.ran_at);
    const posted = po.date;
    const closing = (p.expires_in_days != null && p.expires_in_days <= 2)
      ? `<span class="closing">${p.expires_in_days}d left</span>` : '';
    const tier = p.tier || '';
    return `<tr class="clickable" data-id="${esc(p.id)}" data-tier="${esc(tier)}"
        data-posted="${esc(posted)}" data-tierk="${D.tiers.indexOf(tier)}"
        data-kw="${esc((p.keywords||[]).join(' '))}"
        data-company="${esc(p.company)}" data-title="${esc(p.title)}" data-location="${esc(p.location)}">
      <td class="posted">${posted ? `<span${po.derived ? ' class="derived" title="derived from age"' : ''}>${esc(posted)}</span>` : '<span class="mut">—</span>'}${age ? `<span class="age">${age}</span>` : ''}${closing}</td>
      <td><span class="tier tag tier-${esc(tier)}" title="${esc(p.location||'')}"><i></i>${esc(LOCATION_LABEL[tier] || tier)}</span></td>
      <td class="co" title="${esc(p.company)}">${esc(p.company)}</td>
      <td class="title" title="${esc(p.title)}">${esc(p.title)}</td>
    </tr>`;
  }).join('');
  document.getElementById('rows').innerHTML = rowsHtml;
}

// --- the health page -------------------------------------------------------

function kv(rows){
  return rows.map(([k,v]) => `<div class="k2">${esc(k)}</div><div class="v2">${esc(v)}</div>`).join('');
}

function num(n){
  return (n === null || n === undefined) ? '—' : Number(n).toLocaleString();
}

// The status hero: one glance at what got checked. The headline names the board count,
// and the four tiles carry the numbers worth watching. Deliberately neutral -- a board
// that didn't answer is routine (usually a transient 403/429), not an alarm.
export function renderHero(D){
  const ran = D.ran_at ? D.ran_at.replace('T', ' ').slice(0, 16) : '—';
  document.getElementById('hero').innerHTML =
    '<div>'
    + `<div class="herohead">${esc(Number(D.boards_total || 0).toLocaleString())} boards checked</div>`
    + `<div class="herosub">last scan ${esc(ran)} · ${esc(D.window)}d window · sweep ${esc((D.sweep || {}).verdict || '—')}</div>`
    + '</div>';
  const tiles = [
    ['Answering', num(D.boards_ok) + ' / ' + num(D.boards_total)],
    ["Didn't answer", num(D.boards_failing)],
    ['Live postings', num(D.matched)],
    ['Collected', num(D.collected)],
  ];
  document.getElementById('statgrid').innerHTML = tiles.map(([label, value]) =>
    `<div class="stat"><div class="n">${esc(value)}</div>`
    + `<div class="l">${esc(label)}</div></div>`).join('');
}

export function renderScan(D){
  const s = D.sweep || {};
  document.getElementById('scan').innerHTML = kv([
    ['Ran', D.ran_at ? D.ran_at.replace('T', ' ').slice(0, 19) : '—'],
    ['Boards checked', D.boards_total],
    ['Answering', D.boards_ok],
    ['Live postings', D.matched],
    ['Swept', s.count != null ? s.count : '—'],
    ['Sweep result', s.verdict || '—'],
    ['Source time', D.source_total + 's'],
  ]);
}

export function renderCost(D){
  const cost = Object.entries(D.source_seconds).sort((a, b) => b[1] - a[1]);
  document.getElementById('costrows').innerHTML = cost.length
    ? cost.map(([k, v]) =>
        `<tr><td>${esc(k)}</td><td class="r">${esc(D.sources[k] ?? 0)}</td>`
        + `<td class="r">${v.toFixed(1)}</td></tr>`).join('')
      + `<tr class="total"><td>Total</td><td class="r">${esc(D.collected.toLocaleString())}</td>`
      + `<td class="r">${D.source_total}</td></tr>`
    : '<tr><td colspan="3" class="dim">No timing recorded</td></tr>';
}

export function renderHealth(D){
  const failing = D.boards_failing || 0;
  document.getElementById('failmeta').textContent = failing ? String(failing) : 'none';
  // One plain-English line: how many didn't answer, what kind of failure, when the next
  // retry runs. The full list stays one click away in the disclosure below.
  let note = 'Every board answered the last sweep.';
  if (failing){
    const kinds = [...new Set(D.failing.map(b => b.error_kind).filter(Boolean))];
    const next = D.failing.map(b => b.next_check_at || '').filter(Boolean).sort()[0] || '';
    note = `${failing} board${failing === 1 ? '' : 's'} didn't answer`
      + (kinds.length ? ` (${kinds.join(', ')})` : '')
      + (next ? ` · next retry ${next.slice(0, 16).replace('T', ' ')}` : '')
      + '.';
  }
  document.getElementById('failnote').textContent = note;
  document.getElementById('failrows').innerHTML = D.failing.map(b =>
    `<tr><td>${esc(b.name)}</td><td class="r">${esc(b.consecutive_failures)}</td>`
    + `<td class="dim">${esc(b.error_kind)}</td>`
    + `<td class="dim">${esc(b.last_error)}</td>`
    + `<td class="r">${esc((b.next_check_at||'').slice(0,16).replace('T',' '))}</td></tr>`).join('')
    || '<tr><td colspan="5" class="dim">Nothing here — every board answered.</td></tr>';
}

// --- the detail drawer -----------------------------------------------------

const P = (name, html) => `<div class="dpane" data-pane="${name}">${html}</div>`;

function money(p){
  return p.salary_annual_min
    ? '$' + Math.round(p.salary_annual_min / 1000) + 'k–$' + Math.round(p.salary_annual_max / 1000) + 'k'
    : '—';
}

function briefMissing(d, b){
  if (d.running) return '<p class="spin">Generating the brief…</p>';
  let h = '';
  if (b.status === 'failed') h += `<p class="bad">The last run failed${b.error ? ': ' + esc(b.error) : ''}.</p>`;
  else if (d.stale) h += '<p class="bad">The previous run stopped without finishing.</p>';
  h += '<p class="mut">No brief yet. Generate one to research contacts and get résumé-tailoring advice.</p>';
  h += b.status || d.has_script
    ? '<p><button class="btn" id="gen">Generate brief</button></p>'
    : '<p class="bad">Brief generator unavailable.</p>';
  return h;
}

function footer(b){
  if (b.status !== 'ok') return '';
  const reviewed = b.reviewed === true ? 'reviewed'
    : '<span class="bad">not reviewed</span>' + (b.review_error ? ': ' + esc(b.review_error) : '');
  return `<section class="dfooter"><p class="dimline">${esc(b.generated_at || '')} · ${esc(b.elapsed_s ?? '?')}s · `
    + reviewed + ` · <a href="#" data-regen="${esc(b.posting_id || '')}">Regenerate</a></p></section>`;
}

export function renderDetail(d, ranAt){
  if (d.error) return {title: d.error, sub: '', body: ''};
  const p = d.posting || {}, b = d.brief || {}, t = b.tailoring || {};
  const po = postedOf(p, ranAt);
  const title = p.title || '(untitled)';
  const loc = LOCATION_LABEL[p.tier] || p.tier || '';
  const sub = esc(p.company || '') + ' &nbsp;·&nbsp; ' + esc(loc)
    + ' &nbsp;·&nbsp; score ' + esc(p.score ?? '—')
    + (po.date ? ' &nbsp;·&nbsp; ' + (po.derived ? '~' : '') + esc(po.date) : '');
  const ok = b.status === 'ok';

  const facts = [['Location', p.location],
                 ['Source', (p.source || '') + (p.ats ? ' / ' + p.ats : '')],
                 ['Comp', money(p)],
                 ['ID', p.id]]
    .concat(p.level_ii ? [['Level', 'II']] : [])
    .concat(p.requires_us_work_auth ? [['US work auth', 'Required']] : [])
    .concat(p.below_comp_floor ? [['Comp floor', 'Below']] : []);

  // Summary
  let summary = '<section><div class="kv">'
    + facts.map(([k,v]) => `<div class="k2">${esc(k)}</div><div>${esc(v)}</div>`).join('')
    + '</div>'
    + (p.url ? `<p style="margin-top:12px"><a href="${esc(p.url)}" target="_blank" rel="noreferrer">Open the posting ↗</a></p>` : '')
    + '</section>';
  if (ok){
    if (b.verdict) summary += `<section><p class="verdict">${esc(b.verdict)}</p></section>`;
    if ((b.key_skills || []).length)
      summary += '<section><h4>What decides it</h4><ul class="tight">'
        + b.key_skills.map(x => `<li>${esc(x)}</li>`).join('') + '</ul></section>';
  } else {
    summary += `<section>${briefMissing(d, b)}</section>`;
  }
  if (d.description){
    summary += `<section><details><summary>Posting text (${d.description_length.toLocaleString()} chars${d.description_length > 12000 ? ', trimmed' : ''})</summary>`
      + `<div class="body">${esc(d.description)}</div></details></section>`;
  }
  summary += footer(b);

  // Résumé
  let resume = '';
  if (ok){
    if (t.lead_with) resume += `<section><h4>Lead with</h4><p>${esc(t.lead_with)}</p></section>`;
    if ((t.reword || []).length)
      resume += '<section><h4>Rewrite</h4>' + renderReword(t.reword) + '</section>';
    if ((t.missing || []).length)
      resume += '<section class="dont"><h4>Do not claim</h4><ul class="tight">'
        + t.missing.map(x => `<li>${esc(x)}</li>`).join('') + '</ul></section>';
    if (t.notes) resume += `<section><h4>Notes</h4><p>${esc(t.notes)}</p></section>`;
    resume += `<section><h4>Terms in the posting</h4>${keywordTable(p)}</section>`;
    resume += footer(b);
  } else {
    resume = `<section><p class="mut">${d.running ? 'Generating the brief…' : 'No brief yet — generate one from the Summary tab to get résumé-tailoring advice.'}</p></section>`;
  }

  // Contact
  const known = [].concat(p.emails || [], p.verified_emails || []);
  const contacts = b.contacts || [];
  let contact = '';
  if (d.running) contact += '<section><p class="spin">Searching the web…</p></section>';
  if (known.length)
    contact += '<section><h4>Addresses in the posting</h4>'
      + known.map(a => `<div class="addr"><a href="mailto:${esc(a)}">${esc(a)}</a></div>`).join('') + '</section>';
  if (contacts.length)
    contact += '<section><h4>Researched contacts</h4>'
      + (b.contacts_source ? `<p class="ksub">via ${esc(b.contacts_source)}</p>` : '')
      + contacts.map(contactCard).join('') + '</section>';
  if (!known.length && !contacts.length && !d.running)
    contact += '<section><p class="mut">'
      + (ok ? 'No contact was found for this posting.'
            : 'No brief yet — generate one from the Summary tab, or run the deep search below.')
      + '</p></section>';
  if (b.contact_notes)
    contact += `<section><details><summary>Contact search notes</summary><p>${esc(b.contact_notes)}</p></details></section>`;
  if (!d.running && d.has_script)
    contact += '<section class="deepsec"><h4>Deep search</h4>'
      + '<p class="mut">The brief runs the built-in contact loop. The Hermes agent can take a '
      + 'second, wider pass over the public web — team pages, profiles, and public commit metadata.</p>'
      + `<p><button class="btn ghost" type="button" data-contacts="${esc(p.id)}">Run Hermes contact search</button></p>`
      + '</section>';
  contact += footer(b);

  const body = P('summary', summary) + P('resume', resume) + P('contact', contact);
  return {title, sub, body};
}

// One researched contact as a card: an avatar of their initials, who they are (role and a short
// bio), and how to reach them. `bio` is a newer field on the contact schema -- a brief written
// before it existed carries only `role`, so the role doubles as the bio rather than leaving the
// card thin. Every value is plain text from the brief, so it is escaped once, here.
function initials(name){
  const parts = String(name || '').trim().split(/\s+/).filter(Boolean);
  if (!parts.length) return '?';
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

function contactCard(c){
  const name = c.name || 'Unnamed';
  const hasBio = !!(c.bio && c.bio.trim());
  const role = hasBio ? (c.role || '') : '';
  const bio = hasBio ? c.bio : (c.role || '');
  return '<div class="contact">'
    + '<div class="c-top">'
    + `<span class="c-avatar" aria-hidden="true">${esc(initials(name))}</span>`
    + '<div class="c-id">'
    + `<div class="nm">${esc(name)}`
    + (c.confidence ? `<span class="pill ${esc(c.confidence)}">${esc(c.confidence)}</span>` : '')
    + '</div>'
    + (role ? `<div class="rl">${esc(role)}</div>` : '')
    + '</div></div>'
    + (bio ? `<p class="c-bio">${esc(bio)}</p>` : '')
    + '<div class="c-foot">'
    + (c.email ? `<a class="c-mail" href="mailto:${esc(c.email)}">${esc(c.email)}</a>`
               : '<span class="mut">No published address</span>')
    + (c.source ? `<a class="c-src" href="${esc(c.source)}" target="_blank" rel="noreferrer">source ↗</a>` : '')
    + '</div></div>';
}

function keywordTable(p){
  const have = new Set((p.skills_have || []).map(s => String(s).toLowerCase()));
  const gap  = new Set((p.skills_gap  || []).map(s => String(s).toLowerCase()));
  const terms = p.keywords || [];
  if (!terms.length) return '<p class="mut" style="font-size:12.5px">No keywords</p>';
  const rows = terms.map(k => {
    const low = String(k).toLowerCase();
    const [cls, label] = gap.has(low) ? ['gap', 'Not on résumé']
      : have.has(low) ? ['have', 'On résumé']
      : ['', 'Mentioned'];
    return `<tr><td class="kwt-term">${esc(k)}</td>`
      + `<td class="kwt-st${cls ? ' ' + cls : ''}">${label}</td></tr>`;
  }).join('');
  return '<table class="kwt"><thead><tr><th>Term</th><th class="r">Résumé</th></tr></thead>'
    + `<tbody>${rows}</tbody></table>`;
}

// A reword entry is `{where, before, after, advice, note}`. The left column is the line to
// change, the right is the replacement. An entry with neither still gets a row.
function renderReword(rewords){
  const rows = rewords.map(item => {
    const rw = (typeof item === 'object' && item) ? item : {advice: String(item || '')};
    const where = rw.where || '', before = rw.before || '', advice = rw.advice || '';
    const after = rw.after || '', note = rw.note || '';
    if (!where && !before && !advice && !after && !note) return '';
    const left = before ? `<del>${esc(before)}</del>`
      : (advice ? `<span class="advice">${esc(advice)}</span>` : '<span class="mut">—</span>');
    const right = after ? `<ins>${esc(after)}</ins>` : '<span class="mut">—</span>';
    return '<div class="rw">'
      + (where ? `<div class="rw-where">${esc(where)}</div>` : '')
      + `<div class="rw-cols">${left}<span class="rw-arrow">→</span>${right}</div>`
      + (note ? `<div class="rw-note">${esc(note)}</div>` : '')
      + '</div>';
  }).join('');
  return rows || '<p class="mut" style="font-size:12.5px">Nothing to rewrite.</p>';
}

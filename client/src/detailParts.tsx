// The plain JSX pieces the detail drawer composes, ported one for one from the retired
// client/js/render.js. Every value is plain text from the brief, so it is escaped by JSX.

import type {Brief, Contact, Posting, Reword} from './types';
import {initials, webUrl} from './lib';

// One researched contact: an avatar of their initials, who they are, and how to reach them.
// `bio` is a newer field on the contact schema -- a brief written before it existed carries
// only `role`, so the role doubles as the bio rather than leaving the card thin.
export function ContactCard({c}: {c: Contact}){
  const name = c.name || 'Unnamed';
  const hasBio = !!(c.bio && c.bio.trim());
  const role = hasBio ? (c.role || '') : '';
  const bio = hasBio ? c.bio : (c.role || '');
  return (
    <div class="contact">
      <div class="c-top">
        <span class="c-avatar" aria-hidden="true">{initials(name)}</span>
        <div class="c-id">
          <div class="nm">
            {name}
            {c.confidence ? <span class={'pill ' + (c.confidence || '')}>{c.confidence}</span> : null}
          </div>
          {role ? <div class="rl">{role}</div> : null}
        </div>
      </div>
      {bio ? <p class="c-bio">{bio}</p> : null}
      <div class="c-foot">
        {c.email
          ? <a class="c-mail" href={'mailto:' + c.email}>{c.email}</a>
          : <span class="mut">No published address</span>}
        {webUrl(c.source)
          ? <a class="c-src" href={webUrl(c.source)} target="_blank" rel="noreferrer">source ↗</a>
          : null}
      </div>
    </div>
  );
}

// A reword entry is `{where, before, after, advice, note}`. The left column is the line to
// change, the right is the replacement. An entry with neither still gets a row.
export function RewordBlock({items}: {items: (Reword | string)[]}){
  const rows = items.map((raw, i) => {
    const rw: Reword = (typeof raw === 'object' && raw) ? raw : {advice: String(raw || '')};
    const where = rw.where || '', before = rw.before || '', advice = rw.advice || '';
    const after = rw.after || '', note = rw.note || '';
    if (!where && !before && !advice && !after && !note) return null;
    return (
      <div class="rw" key={i}>
        {where ? <div class="rw-where">{where}</div> : null}
        <div class="rw-cols">
          {before
            ? <del>{before}</del>
            : advice ? <span class="advice">{advice}</span> : <span class="mut">—</span>}
          <span class="rw-arrow">→</span>
          {after ? <ins>{after}</ins> : <span class="mut">—</span>}
        </div>
        {note ? <div class="rw-note">{note}</div> : null}
      </div>
    );
  }).filter(Boolean);
  if (!rows.length) return <p class="mut" style="font-size:12.5px">Nothing to rewrite.</p>;
  return <>{rows}</>;
}

export function KeywordTable({p}: {p: Posting}){
  const have = new Set((p.skills_have || []).map(s => String(s).toLowerCase()));
  const gap = new Set((p.skills_gap || []).map(s => String(s).toLowerCase()));
  const terms = p.keywords || [];
  if (!terms.length) return <p class="mut" style="font-size:12.5px">No keywords</p>;
  return (
    <table class="kwt">
      <thead><tr><th>Term</th><th class="r">Résumé</th></tr></thead>
      <tbody>
        {terms.map((k, i) => {
          const low = String(k).toLowerCase();
          const cls = gap.has(low) ? 'gap' : have.has(low) ? 'have' : '';
          const label = gap.has(low) ? 'Not on résumé' : have.has(low) ? 'On résumé' : 'Mentioned';
          return (
            <tr key={i}>
              <td class="kwt-term">{k}</td>
              <td class={'kwt-st' + (cls ? ' ' + cls : '')}>{label}</td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

// The one-line provenance footer under each pane of a generated brief.
export function BriefFooter({b}: {b: Brief}){
  if (b.status !== 'ok') return null;
  const reviewed = b.reviewed === true
    ? 'reviewed'
    : <><span class="bad">not reviewed</span>{b.review_error ? ': ' + b.review_error : ''}</>;
  const when = b.generated_at ? b.generated_at.slice(0, 10) : '';
  return (
    <section class="dfooter">
      <p class="dimline">
        {when ? <>Generated {when} · </> : null}{b.elapsed_s ?? '?'}s · {reviewed}
        {b.posting_id ? <> · <a href="#" data-regen={b.posting_id}>Regenerate</a></> : null}
      </p>
    </section>
  );
}

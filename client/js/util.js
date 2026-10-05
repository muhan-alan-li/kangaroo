// Small helpers shared by the render modules.

export function esc(s){
  return (s == null ? '' : String(s)).replace(/[&<>"']/g, c =>
    ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
}

// Workday's board API publishes only a relative age ("Posted Today"), never an absolute date,
// so about two thirds of the shortlist had no date and sorted to the bottom. age_days is exact
// though, and every row records the moment the scan that produced it ran, so the date is
// derivable -- good to the day.
export function postedOf(p, ranAt){
  if (p.posted) return {date: p.posted, derived: false};
  if (p.posted_derived) return {date: p.posted_derived, derived: true};
  const stamp = p.scanned_at || ranAt;
  if (stamp && p.age_days !== null && p.age_days !== undefined){
    const t = new Date(new Date(stamp).getTime() - p.age_days * 86400000);
    return {date: t.toISOString().slice(0, 10), derived: true};
  }
  return {date: '', derived: false};
}

export function kwList(v, cls){
  return (v || []).length
    ? v.map(x => `<span class="k ${cls}">${esc(x)}</span>`).join('')
    : '<span class="mut">—</span>';
}

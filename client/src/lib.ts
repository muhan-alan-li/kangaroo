// Small helpers shared by the views. Pure functions only.

import type {Posting} from './types';

// The canonical location buckets. The scan's `tier` already aggregates every spelling of a
// place, so this is a rename of the ranking field, not a second classifier.
export const LOCATION_LABEL: Record<string, string> = {
  vancouver: 'Vancouver', remote_canada: 'Remote Canada', toronto: 'Toronto',
  seattle: 'Seattle', remote_us: 'Remote US', remote_unspecified: 'Remote',
};

export const tierLabel = (tier: string) => LOCATION_LABEL[tier] || tier || '';

// Only web URLs are allowed into an href. The scan's feeds are untrusted input, and a
// `javascript:` URL would otherwise run in this page when the reader clicked the row.
export const webUrl = (url?: string | null): string =>
  (typeof url === 'string' && /^https?:\/\//i.test(url)) ? url : '';

// Workday's board API publishes only a relative age ("Posted Today"), never an absolute date,
// so the date is derivable from age_days and the moment the scan ran -- good to the day.
export function postedOf(p: Posting, ranAt?: string): {date: string; derived: boolean} {
  if (p.posted) return {date: p.posted, derived: false};
  if (p.posted_derived) return {date: p.posted_derived, derived: true};
  const stamp = p.scanned_at || ranAt || '';
  const age = p.age_days;
  const ms = stamp ? Date.parse(stamp) : NaN;
  if (Number.isFinite(ms) && age !== null && age !== undefined && Number.isFinite(age)){
    const t = new Date(ms - age * 86400000);
    return {date: t.toISOString().slice(0, 10), derived: true};
  }
  return {date: '', derived: false};
}

export function money(p: Posting): string {
  const min = p.salary_annual_min;
  if (min === null || min === undefined) return '—';
  const lo = '$' + Math.round(min / 1000) + 'k';
  const max = p.salary_annual_max;
  return (max === null || max === undefined || max === min)
    ? lo
    : lo + '–$' + Math.round(max / 1000) + 'k';
}

export function initials(name?: string): string {
  const parts = String(name || '').trim().split(/\s+/).filter(Boolean);
  if (!parts.length) return '?';
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

// Dark-by-default theme toggle. The choice persists in localStorage so it survives reloads and
// carries over between the two pages; the initial theme is set by inline script in the HTML.
export function initThemeToggle(){
  const root = document.documentElement;
  const toggle = document.getElementById('themeToggle');
  const paint = () => {
    if (toggle) toggle.textContent = root.dataset.theme === 'light' ? 'dark' : 'light';
  };
  toggle?.addEventListener('click', () => {
    root.dataset.theme = root.dataset.theme === 'light' ? 'dark' : 'light';
    try { localStorage.setItem('jobhunt-theme', root.dataset.theme!); } catch { /* private mode */ }
    paint();
  });
  paint();
}

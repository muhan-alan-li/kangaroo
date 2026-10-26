// The only place the client talks to the server. Everything else renders or filters what these
// return. Mirrors the retired client/js/api.js one for one.

import type {AppliedResponse, Dashboard, Detail} from './types';

async function getJSON<T>(url: string): Promise<T> {
  const r = await fetch(url, {headers: {'Accept': 'application/json'}, cache: 'no-store'});
  if (!r.ok) throw new Error(`${url} -> HTTP ${r.status}`);
  return r.json() as Promise<T>;
}

async function postJSON<T>(url: string, body?: unknown): Promise<T> {
  const r = await fetch(url, body === undefined
    ? {method: 'POST', headers: {'Accept': 'application/json'}}
    : {method: 'POST', headers: {'Accept': 'application/json', 'Content-Type': 'application/json'},
       body: JSON.stringify(body)});
  if (!r.ok) throw new Error(`${url} -> HTTP ${r.status}`);
  return r.json() as Promise<T>;
}

export const fetchDashboard = () => getJSON<Dashboard>('/api/dashboard');
export const fetchPosting = (id: string) => getJSON<Detail>('/api/posting/' + encodeURIComponent(id));
export const triggerBrief = (id: string) =>
  postJSON<Record<string, unknown>>('/api/brief/' + encodeURIComponent(id));
// Second level: the Hermes agent contact search, merged into the existing brief server-side.
export const triggerContacts = (id: string) =>
  postJSON<Record<string, unknown>>('/api/deep-contacts/' + encodeURIComponent(id));
// A write the browser can fire at click time: it must not delay the apply link's own navigation,
// so the caller opens the posting first and lets this land in the background.
export const markApplied = (id: string) =>
  postJSON<AppliedResponse>('/api/applied/' + encodeURIComponent(id));
// One request for the whole selection, so a bulk removal is a single write on the server.
export const removeApplied = (ids: string[]) =>
  postJSON<AppliedResponse>('/api/applied/remove', {ids});

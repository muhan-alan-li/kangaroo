// The only place the client talks to the server. Everything else renders or filters what
// these return.

async function getJSON(url){
  const r = await fetch(url, {headers: {'Accept': 'application/json'}, cache: 'no-store'});
  if (!r.ok) throw new Error(`${url} -> HTTP ${r.status}`);
  return r.json();
}

export const fetchDashboard = () => getJSON('/api/dashboard');

export const fetchPosting = (id) => getJSON('/api/posting/' + encodeURIComponent(id));

export async function triggerBrief(id){
  const r = await fetch('/api/brief/' + encodeURIComponent(id),
                        {method: 'POST', headers: {'Accept': 'application/json'}});
  if (!r.ok) throw new Error(`/api/brief/${id} -> HTTP ${r.status}`);
  return r.json();
}

// Second level: the Hermes agent contact search, merged into the existing brief server-side.
export async function triggerContacts(id){
  const r = await fetch('/api/deep-contacts/' + encodeURIComponent(id),
                        {method: 'POST', headers: {'Accept': 'application/json'}});
  if (!r.ok) throw new Error(`/api/deep-contacts/${id} -> HTTP ${r.status}`);
  return r.json();
}

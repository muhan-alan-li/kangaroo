// The detail drawer. Clicking a row opens the posting: its own facts, the text the scan
// cached, and the brief. Nothing is generated until the button is pressed, because generating
// one is a model call (and, in the agent engine, a web research run).
import {fetchPosting, triggerBrief, triggerContacts} from './api.js?v=20261026a';
import {renderDetail} from './render.js?v=20261026a';
import {esc} from './util.js?v=20261026a';

export function initDetail(ranAt){
  const drawer = document.getElementById('drawer');
  const scrim  = document.getElementById('scrim');
  const dtabs  = document.getElementById('dtabs');
  let current = null, briefTimer = null, activeTab = 'summary';

  function closeDrawer(){
    drawer.classList.remove('on'); scrim.classList.remove('on');
    drawer.setAttribute('aria-hidden','true');
    current = null;
    if (briefTimer){ clearInterval(briefTimer); briefTimer = null; }
  }

  document.getElementById('dclose').onclick = closeDrawer;
  scrim.onclick = closeDrawer;
  document.addEventListener('keydown', e => { if (e.key === 'Escape') closeDrawer(); });

  function openDetail(pid){
    current = pid;
    activeTab = 'summary';
    dtabs.classList.remove('on');
    drawer.classList.add('on'); scrim.classList.add('on');
    drawer.setAttribute('aria-hidden','false');
    document.getElementById('dtitle').textContent = 'loading…';
    document.getElementById('dsub').textContent = '';
    document.getElementById('dbody').innerHTML = '';
    drawer.scrollTop = 0;
    loadDetail(pid);
  }

  async function loadDetail(pid){
    let d;
    try {
      d = await fetchPosting(pid);
    } catch (err){
      document.getElementById('dbody').innerHTML =
        '<section><p class="bad">could not load: ' + esc(err.message) + '</p></section>';
      return null;
    }
    if (current !== pid) return null;            // the user moved to another posting
    renderLoaded(d);
    return d;
  }

  function renderLoaded(d){
    const view = renderDetail(d, ranAt);
    document.getElementById('dtitle').textContent = view.title;
    document.getElementById('dsub').innerHTML = view.sub;
    document.getElementById('dbody').innerHTML = view.body;
    dtabs.classList.add('on');
    applyTab(activeTab);
    const gen = document.getElementById('gen');
    if (gen) gen.onclick = () => startBrief((d.posting || {}).id);
  }

  // Tabs, not one long scroll: only the chosen pane is shown. The active tab is kept in
  // `activeTab` so the 7s poll's re-render does not bounce the user back to Brief.
  function applyTab(tab){
    activeTab = tab;
    dtabs.querySelectorAll('button').forEach(b =>
      b.classList.toggle('on', b.dataset.tab === tab));
    document.querySelectorAll('#dbody .dpane').forEach(pane =>
      pane.classList.toggle('on', pane.dataset.pane === tab));
  }
  dtabs.addEventListener('click', e => {
    const b = e.target.closest('button[data-tab]');
    if (b) applyTab(b.dataset.tab);
  });

  async function startBrief(pid){
    const btn = document.getElementById('gen');
    if (btn){ btn.disabled = true; btn.textContent = 'starting…'; }
    try {
      await triggerBrief(pid);
    } catch (e){ /* the state read below will show whatever happened */ }
    loadDetail(pid);
    watch(pid);
  }

  // Second level: the Hermes agent contact search. Same polling contract as the brief.
  async function startContacts(pid){
    const btn = document.querySelector('[data-contacts]');
    if (btn){ btn.disabled = true; btn.textContent = 'searching…'; }
    try {
      await triggerContacts(pid);
    } catch (e){ /* the poll below shows whatever happened */ }
    loadDetail(pid);
    watch(pid);
  }

  // Poll until the run finishes, one way or another. The stop test is the server's `running`
  // flag, NOT the presence of the Generate button: the button is absent while a run is in flight
  // too, so testing for it stopped the poll on its first tick and the panel never showed the
  // result. The server marks a run stale after 15 minutes, so this cannot spin forever even if
  // the generator dies.
  function watch(pid){
    if (briefTimer) clearInterval(briefTimer);
    let ticks = 0;
    briefTimer = setInterval(async () => {
      ticks++;
      if (current !== pid){ clearInterval(briefTimer); briefTimer = null; return; }
      const d = await loadDetail(pid);
      if ((d && !d.running) || ticks > 100){
        clearInterval(briefTimer); briefTimer = null;
      }
    }, 7000);
    return briefTimer;
  }

  // Delegated, so it keeps working after the table is re-rendered or re-sorted.
  document.getElementById('rows').addEventListener('click', e => {
    if (e.target.closest('a')) return;              // let the posting link and mailto through
    const tr = e.target.closest('tr');
    if (tr && tr.dataset.id) openDetail(tr.dataset.id);
  });

  // A brief can be regenerated in place. Worth having even though it costs an agent run: a brief
  // written before the critic existed, or one whose posting has since changed, is otherwise stuck.
  document.getElementById('dbody').addEventListener('click', e => {
    const deep = e.target.closest('[data-contacts]');
    if (deep && deep.dataset.contacts){ startContacts(deep.dataset.contacts); return; }
    const a = e.target.closest('[data-regen]');
    if (!a || !a.dataset.regen) return;
    e.preventDefault();
    startBrief(a.dataset.regen);
  });
}

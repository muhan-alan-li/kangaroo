// Client-side filtering, sorting and paging of the shortlist. Everything is already in the DOM,
// so these are instant and survive on a phone with no connection to the server.

// Newest posting first is the default view: the freshest requisition is the one still worth a
// look, and a posting's age is what makes it stale. Anything undated sorts last in either
// direction (see applySort), so it can never head a newest-first list.
const DEFAULT_SORT = {key: 'posted', dir: -1};
const DEFAULT_PAGE_SIZE = 25;

export function initTable(D){
  let activeTier = null, needle = '', mailOnly = false, gapsOnly = false;
  let sortKey = DEFAULT_SORT.key, sortDir = DEFAULT_SORT.dir;
  let page = 1, pageSize = DEFAULT_PAGE_SIZE;

  const body = document.getElementById('rows');
  const empty = document.getElementById('empty');

  function matches(tr){
    const okT = !activeTier || tr.dataset.tier === activeTier;
    const okM = !mailOnly || tr.dataset.mail === '1';
    const okG = !gapsOnly || tr.dataset.gap === '1';
    const hay = (tr.dataset.company + ' ' + tr.dataset.title + ' ' + tr.dataset.location
                 + ' ' + tr.dataset.kw).toLowerCase();
    const okQ = !needle || hay.includes(needle);
    return okT && okQ && okM && okG;
  }

  // Filter, then slice out the current page. Sorting has already reordered the DOM, so the
  // surviving rows arrive in the order the reader chose. The "#" column is renumbered over the
  // whole filtered set (not per page), so it stays a stable position under the active sort.
  function apply(){
    const hits = [];
    for (const tr of body.rows) if (matches(tr)) hits.push(tr);

    const pages = Math.max(1, Math.ceil(hits.length / pageSize));
    if (page > pages) page = pages;
    if (page < 1) page = 1;
    const start = (page - 1) * pageSize, end = Math.min(start + pageSize, hits.length);

    const keep = new Set(hits.slice(start, end));
    body.querySelectorAll('tr').forEach(tr => tr.classList.toggle('flt', !keep.has(tr)));
    hits.forEach((tr, i) => { tr.cells[0].textContent = i + 1; });

    empty.style.display = hits.length ? 'none' : 'block';
    renderPager(hits.length, pages, start, end);
  }

  function renderPager(total, pages, start, end){
    document.getElementById('pginfo').textContent =
      total ? `showing ${start + 1}\u2013${end} of ${total}` : '';
    document.getElementById('pgnum').textContent = total ? `page ${page} / ${pages}` : '';
    document.getElementById('pgprev').disabled = page <= 1;
    document.getElementById('pgnext').disabled = page >= pages;
    document.getElementById('pager').style.display = total ? 'flex' : 'none';
  }

  function applySort(){
    const k = sortKey;
    if (k){
      const rows = [...body.rows];
      rows.sort((a, b) => {
        if (k === 'posted'){
          // An ISO date sorts chronologically as a string. An undated posting goes last
          // whichever direction is active: "no publish date" is not "published in 1970", so it
          // must never top a newest-first list. Rippling and BambooHR publish no date at all.
          const x = a.dataset.posted || '', y = b.dataset.posted || '';
          if (x && y) return x.localeCompare(y) * sortDir;
          if (!x && !y) return 0;
          return x ? -1 : 1;
        }
        if (k === 'tier') return (+a.dataset.tierk - +b.dataset.tierk) * sortDir;
        if (k === 'score' || k === 'age' || k === 'rank'){
          const attr = (k === 'rank') ? 'rank' : k;
          return (+a.dataset[attr] - +b.dataset[attr]) * sortDir;
        }
        return ((a.dataset[k] || '').localeCompare(b.dataset[k] || '')) * sortDir;
      });
      rows.forEach(r => body.appendChild(r));
    }
    apply();
  }

  document.getElementById('q').addEventListener('input', e => {
    needle = e.target.value.toLowerCase(); page = 1; apply();
  });
  // Location is a single-select dropdown now, not a row of chips.
  document.getElementById('locfilter').addEventListener('change', e => {
    activeTier = e.target.value || null; page = 1; apply();
  });
  document.getElementById('mailonly').addEventListener('click', e => {
    mailOnly = !mailOnly; e.target.classList.toggle('on', mailOnly); page = 1; apply();
  });
  document.getElementById('gaps').addEventListener('click', e => {
    gapsOnly = !gapsOnly; e.target.classList.toggle('on', gapsOnly); page = 1; apply();
  });

  document.getElementById('pgprev').addEventListener('click', () => {
    page--; apply();
  });
  document.getElementById('pgnext').addEventListener('click', () => {
    page++; apply();
  });
  document.getElementById('pgsize').addEventListener('change', e => {
    pageSize = +e.target.value; page = 1; apply();
  });

  // The direction a column takes on its FIRST click. Newest-first is what "date posted" should
  // do; highest-score-first is what the score column should do. Everything else starts ascending.
  const FIRST_DIR = {posted: -1, score: -1};
  document.querySelectorAll('th[data-s]').forEach(th => th.addEventListener('click', () => {
    const k = th.dataset.s;
    sortDir = (sortKey === k) ? -sortDir : (FIRST_DIR[k] || 1);
    sortKey = k;
    page = 1;
    document.querySelectorAll('th[data-s]').forEach(h => {
      h.classList.toggle('sorted', h === th);
      if (h !== th) h.removeAttribute('data-dir');
      else h.dataset.dir = sortDir > 0 ? 'asc' : 'desc';
    });
    applySort();
  }));

  // Paint the default sort's indicator before the reader touches anything.
  const def = document.querySelector(`th[data-s="${DEFAULT_SORT.key}"]`);
  if (def){
    def.classList.add('sorted');
    def.dataset.dir = DEFAULT_SORT.dir > 0 ? 'asc' : 'desc';
  }
  applySort();
}

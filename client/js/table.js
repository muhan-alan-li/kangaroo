// Client-side filtering, sorting and paging of the shortlist. Everything is already in the DOM,
// so these are instant and survive on a phone with no connection to the server.

// Newest posting first is the default view: the freshest requisition is the one still worth a
// look, and a posting's age is what makes it stale. Anything undated sorts last in either
// direction (see applySort), so it can never head a newest-first list.
const DEFAULT_SORT = {key: 'posted', dir: -1};
const DEFAULT_PAGE_SIZE = 10;

export function initTable(D){
  let activeTier = null, needle = '';
  let sortKey = DEFAULT_SORT.key, sortDir = DEFAULT_SORT.dir;
  let page = 1, pageSize = DEFAULT_PAGE_SIZE;

  const body = document.getElementById('rows');
  const empty = document.getElementById('empty');

  function matches(tr){
    const okT = !activeTier || tr.dataset.tier === activeTier;
    const hay = (tr.dataset.company + ' ' + tr.dataset.title + ' ' + tr.dataset.location
                 + ' ' + tr.dataset.kw).toLowerCase();
    const okQ = !needle || hay.includes(needle);
    return okT && okQ;
  }

  // Filter, then slice out the current page. Sorting has already reordered the DOM, so the
  // surviving rows arrive in the order the reader chose.
  function apply(){
    const hits = [];
    for (const tr of body.rows) if (matches(tr)) hits.push(tr);

    const pages = Math.max(1, Math.ceil(hits.length / pageSize));
    if (page > pages) page = pages;
    if (page < 1) page = 1;
    const start = (page - 1) * pageSize, end = Math.min(start + pageSize, hits.length);

    const keep = new Set(hits.slice(start, end));
    body.querySelectorAll('tr').forEach(tr => tr.classList.toggle('flt', !keep.has(tr)));

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

  document.getElementById('pgprev').addEventListener('click', () => {
    page--; apply();
  });
  document.getElementById('pgnext').addEventListener('click', () => {
    page++; apply();
  });
  document.getElementById('pgsize').addEventListener('change', e => {
    const v = e.target.value;
    try { localStorage.setItem('jobhunt-pagesize', v); } catch (err) {}
    if (v === 'fit') fitToViewport();
    else { pageSize = +v; pinWrapper(); }
    page = 1; apply();
  });

  // The direction a column takes on its FIRST click. Newest-first is what "date posted" should
  // do; everything else starts ascending.
  const FIRST_DIR = {posted: -1};
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

  // Fit the page to the viewport: rows fill exactly the space between the table head
  // and the pager, so the whole page lands with no scrollbar. A manual rows choice
  // disables the fit until "Fit" is picked again; the choice persists.
  const FIT_MIN = 3, FIT_MAX = 30;
  let storedSize = 'fit';
  try { storedSize = localStorage.getItem('jobhunt-pagesize') || 'fit'; } catch (e) {}
  document.getElementById('pgsize').value = storedSize;

  function measured(){
    const toolbar = document.querySelector('.toolbar');
    const thead = document.querySelector('.shortlist thead');
    const pager = document.getElementById('pager');
    const tr = body.rows[0];
    return {
      frame: 72 + 20 + 28, // topbar + page top pad + page bottom pad
      toolbar: toolbar ? toolbar.offsetHeight : 60,
      thead: thead ? thead.getBoundingClientRect().height : 48,
      pager: pager ? pager.offsetHeight : 49,
      row: tr ? tr.getBoundingClientRect().height : 52,
    };
  }
  // Reserve exactly one page of space so the pager never moves under filtering.
  function pinWrapper(){
    const m = measured();
    const wrap = document.querySelector('.tablewrap');
    if (wrap) wrap.style.minHeight = Math.ceil(m.thead + pageSize * m.row + 2) + 'px';
  }
  function fitToViewport(){
    const m = measured();
    pageSize = Math.min(FIT_MAX, Math.max(FIT_MIN,
      Math.floor((window.innerHeight - m.frame - m.toolbar - m.thead - m.pager) / m.row)));
    pinWrapper();
    apply();
    // The estimate above can't see card borders/margins or sub-pixel rounding, which are
    // exactly what leave a phantom few pixels of scroll. Trim a row at a time until the
    // document truly fits; bounded so a tiny viewport just scrolls a little.
    let guard = 0;
    while (document.documentElement.scrollHeight > window.innerHeight + 1
           && pageSize > FIT_MIN && guard++ < 10){
      pageSize--; pinWrapper(); apply();
    }
  }

  if (storedSize === 'fit') fitToViewport();
  else { pageSize = +storedSize || DEFAULT_PAGE_SIZE; pinWrapper(); }

  // Row heights shift once the webfonts arrive; refit then, and on window resizes.
  if (document.fonts && document.fonts.ready){
    document.fonts.ready.then(() => {
      if (document.getElementById('pgsize').value === 'fit'){ fitToViewport(); apply(); }
      else pinWrapper();
    });
  }
  let resizeT = null;
  window.addEventListener('resize', () => {
    clearTimeout(resizeT);
    resizeT = setTimeout(() => {
      if (document.getElementById('pgsize').value === 'fit'){ fitToViewport(); apply(); }
    }, 150);
  });

  applySort();
}

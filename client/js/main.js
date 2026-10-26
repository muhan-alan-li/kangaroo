// Entry point for the postings page: fetch the dashboard payload once, render the list, then
// hand the table and the drawer their event handlers.
import {fetchDashboard} from './api.js?v=20261026a';
import {renderLocations, renderRows} from './render.js?v=20261026a';
import {initTable} from './table.js?v=20261026a';
import {initDetail} from './detail.js?v=20261026a';
import {initPrefs} from './prefs.js?v=20261026a';

initPrefs();

(async function main(){
  let D;
  try {
    D = await fetchDashboard();
  } catch (err){
    document.getElementById('topmeta').textContent = 'could not load';
    return;
  }
  document.getElementById('topmeta').textContent = D.ran_at ? D.ran_at.slice(0, 10) : '';
  renderLocations(D);
  renderRows(D);
  initTable(D);
  initDetail(D.ran_at);
})();

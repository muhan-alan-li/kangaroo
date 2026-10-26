// Entry point for the scan-health page. Same payload as the postings page; only the
// scan/board sections are rendered here.
import {fetchDashboard} from './api.js?v=20261026a';
import {renderHero, renderScan, renderCost, renderHealth} from './render.js?v=20261026a';
import {initPrefs} from './prefs.js?v=20261026a';

initPrefs();

(async function main(){
  let D;
  try {
    D = await fetchDashboard();
  } catch (err){
    document.getElementById('scan').innerHTML = '<div>could not load: ' + err.message + '</div>';
    return;
  }
  document.getElementById('topmeta').textContent = D.ran_at ? D.ran_at.slice(0, 10) : '';
  renderHero(D);
  renderScan(D);
  renderCost(D);
  renderHealth(D);
})();

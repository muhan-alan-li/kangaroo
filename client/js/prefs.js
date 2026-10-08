// Dark-by-default theme toggle. The choice persists in localStorage so it survives
// reloads and carries over to the health page.
export function initPrefs(){
  const root = document.documentElement;

  const toggle = document.getElementById('themeToggle');
  const paintToggle = () =>
    { if (toggle) toggle.textContent = root.dataset.theme === 'light' ? 'dark' : 'light'; };
  if (toggle) toggle.addEventListener('click', () => {
    root.dataset.theme = root.dataset.theme === 'light' ? 'dark' : 'light';
    try { localStorage.setItem('jobhunt-theme', root.dataset.theme); } catch (e) {}
    paintToggle();
  });
  paintToggle();
}

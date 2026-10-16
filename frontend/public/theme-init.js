// Apply the saved theme before first paint to avoid a light/dark flash.
// Mirrors applyTheme() in src/stores/theme.store.ts. Kept as an external file
// (not inline) so the app works under a strict `script-src 'self'` CSP.
(function () {
  var theme = 'system';
  try {
    theme = localStorage.getItem('parcel_theme') || 'system';
  } catch (e) {}
  var dark =
    theme === 'dark' ||
    (theme !== 'light' &&
      window.matchMedia &&
      window.matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.classList.toggle('dark', dark);
  document.documentElement.style.colorScheme = dark ? 'dark' : 'light';
})();

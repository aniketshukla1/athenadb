// Runs before the first paint, so a saved theme never flashes the wrong one.
(function () {
  try {
    var theme = localStorage.getItem('athenadb-theme')
    if (theme === 'light' || theme === 'dark') document.documentElement.dataset.theme = theme
  } catch (e) { /* storage blocked: follow the system */ }
})()

try {
  const theme = localStorage.getItem("wereader-site-theme");
  if (theme) document.documentElement.dataset.theme = theme;
} catch {}

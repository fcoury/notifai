const { getCurrentWindow } = window.__TAURI__.window;

// Open external links
document.getElementById('github-link').addEventListener('click', async (e) => {
  e.preventDefault();
  // Use shell.open when available, for now just prevent default
  // The GitHub URL would be opened here
});

// Close on Escape key
document.addEventListener('keydown', async (e) => {
  if (e.key === 'Escape') {
    e.preventDefault();
    const window = getCurrentWindow();
    await window.close();
  }
});

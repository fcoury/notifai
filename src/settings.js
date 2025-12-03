const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;

// DOM elements
const form = document.getElementById('settings-form');
const errorsDiv = document.getElementById('errors');
const resetBtn = document.getElementById('reset-btn');
const cancelBtn = document.getElementById('cancel-btn');
const saveBtn = document.getElementById('save-btn');
const notificationsEnabled = document.getElementById('notifications-enabled');

// Track if form is dirty
let originalSettings = null;

// Default values
const DEFAULTS = {
  refresh_interval_minutes: 15,
  threshold_under_budget: 85,
  threshold_on_track: 115,
  notifications_enabled: true,
  notify_approaching_percent: 100,
  notify_over_budget_percent: 115
};

// Load settings on page load
async function loadSettings() {
  try {
    const settings = await invoke('get_settings');
    populateForm(settings);
    originalSettings = { ...settings };
  } catch (error) {
    console.error('Failed to load settings:', error);
    populateForm(DEFAULTS);
    originalSettings = { ...DEFAULTS };
  }
}

function populateForm(settings) {
  document.getElementById('refresh-interval').value = settings.refresh_interval_minutes;
  document.getElementById('under-budget').value = settings.threshold_under_budget;
  document.getElementById('on-track').value = settings.threshold_on_track;
  document.getElementById('notifications-enabled').checked = settings.notifications_enabled;
  document.getElementById('notify-approaching').value = settings.notify_approaching_percent;
  document.getElementById('notify-over').value = settings.notify_over_budget_percent;

  updateNotificationFieldsState();
}

function getFormValues() {
  return {
    refresh_interval_minutes: parseInt(document.getElementById('refresh-interval').value),
    threshold_under_budget: parseFloat(document.getElementById('under-budget').value),
    threshold_on_track: parseFloat(document.getElementById('on-track').value),
    notifications_enabled: document.getElementById('notifications-enabled').checked,
    notify_approaching_percent: parseFloat(document.getElementById('notify-approaching').value),
    notify_over_budget_percent: parseFloat(document.getElementById('notify-over').value)
  };
}

function updateNotificationFieldsState() {
  const enabled = notificationsEnabled.checked;
  document.querySelectorAll('.notification-field').forEach(el => {
    el.classList.toggle('disabled', !enabled);
  });
}

function validateForm() {
  const values = getFormValues();
  const errors = [];

  if (values.threshold_under_budget < 1 || values.threshold_under_budget > 99) {
    errors.push('Under budget threshold must be between 1 and 99%');
  }

  if (values.threshold_on_track < 2 || values.threshold_on_track > 200) {
    errors.push('On track threshold must be between 2 and 200%');
  }

  if (values.threshold_under_budget >= values.threshold_on_track) {
    errors.push('Under budget threshold must be less than over budget threshold');
  }

  if (values.notify_approaching_percent < 1 || values.notify_approaching_percent > 200) {
    errors.push('Approaching notification threshold must be between 1 and 200%');
  }

  if (values.notify_over_budget_percent < 1 || values.notify_over_budget_percent > 200) {
    errors.push('Over budget notification threshold must be between 1 and 200%');
  }

  if (values.notify_over_budget_percent < values.notify_approaching_percent) {
    errors.push('Over budget notification must be >= approaching notification');
  }

  return errors;
}

function showErrors(errors) {
  if (errors.length === 0) {
    errorsDiv.hidden = true;
    return;
  }

  errorsDiv.innerHTML = `<ul>${errors.map(e => `<li>${e}</li>`).join('')}</ul>`;
  errorsDiv.hidden = false;
}

// Event handlers
form.addEventListener('change', () => {
  showErrors(validateForm());
});

notificationsEnabled.addEventListener('change', updateNotificationFieldsState);

form.addEventListener('submit', async (e) => {
  e.preventDefault();

  const errors = validateForm();
  if (errors.length > 0) {
    showErrors(errors);
    return;
  }

  try {
    const settings = getFormValues();
    await invoke('save_settings_cmd', { newSettings: settings });
    originalSettings = { ...settings };
    showErrors([]);

    // Close window after successful save
    const window = getCurrentWindow();
    await window.close();
  } catch (error) {
    showErrors([`Failed to save: ${error}`]);
  }
});

resetBtn.addEventListener('click', () => {
  populateForm(DEFAULTS);
  showErrors([]);
});

cancelBtn.addEventListener('click', async () => {
  const window = getCurrentWindow();
  await window.close();
});

// Keyboard shortcuts
document.addEventListener('keydown', async (e) => {
  if (e.key === 'Escape') {
    e.preventDefault();
    cancelBtn.click();
  } else if (e.key === 'Enter' && e.metaKey) {
    e.preventDefault();
    saveBtn.click();
  }
});

// Initialize
loadSettings();

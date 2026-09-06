const appearance = document.querySelector('#appearance');
const preferenceKey = 'shep.website.appearance';
try {
  const saved = localStorage.getItem(preferenceKey);
  if (['light', 'dark', 'system'].includes(saved)) appearance.value = saved;
} catch { /* A blocked preference store must not prevent navigation. */ }

function applyAppearance() {
  document.documentElement.dataset.theme = appearance.value;
}
applyAppearance();
appearance.addEventListener('change', () => {
  applyAppearance();
  try { localStorage.setItem(preferenceKey, appearance.value); } catch { /* Session-only appearance. */ }
});

// A hint only: every platform remains visible and selectable. Android comes
// before Linux, and desktop-mode iPads come before macOS.
function suggestedPlatform() {
  const ua = navigator.userAgent;
  const platform = navigator.userAgentData?.platform ?? navigator.platform ?? '';
  if (/Android/i.test(`${ua} ${platform}`)) return 'android';
  if (/iPhone|iPad|iPod/i.test(ua) || (/Mac/i.test(platform) && navigator.maxTouchPoints > 1)) return 'ios';
  if (/Windows|Win32/i.test(`${ua} ${platform}`)) return 'windows';
  if (/CrOS/i.test(ua)) return 'browser';
  if (/Mac/i.test(`${ua} ${platform}`)) return 'macos';
  if (/Linux/i.test(`${ua} ${platform}`)) return 'linux';
  return null;
}

const suggested = suggestedPlatform();
const labels = {
  linux: ['Install on Linux', 'Linux is available to build and install for your user.'],
  macos: ['Shep for macOS', 'macOS builds are still in development. See availability below.'],
  windows: ['Shep for Windows', 'Windows builds are still in development. See availability below.'],
  android: ['Shep for Android', 'The Android app is in development. Google Play availability is listed below.'],
  ios: ['Shep for iPhone & iPad', 'The iPhone and iPad app is in development. App Store availability is listed below.'],
  browser: ['Shep browser beta', 'An invite-only browser beta with Google sign-in is planned. Access is not open yet.'],
};
if (suggested) {
  document.querySelector('#suggested-label').textContent = labels[suggested][0];
  document.querySelector('#suggested-install').href = `#${suggested}`;
  document.querySelector('#install-note').textContent = labels[suggested][1];
  document.querySelector(`#${suggested}`).dataset.suggested = 'true';
}

const copyButton = document.querySelector('#copy-install');
copyButton.hidden = false;
copyButton.addEventListener('click', async () => {
  const status = document.querySelector('#copy-status');
  try {
    await navigator.clipboard.writeText(document.querySelector('#install-command').textContent);
    status.textContent = 'Installation commands copied.';
  } catch {
    status.textContent = 'Copy is unavailable. Select and copy the commands above.';
  }
});

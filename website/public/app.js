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

// Android comes before Linux, and desktop-mode iPads come before macOS.
// ChromeOS installs through its Linux environment.
function detectedPlatform() {
  const ua = navigator.userAgent;
  const platform = navigator.userAgentData?.platform ?? navigator.platform ?? '';
  if (/Android/i.test(`${ua} ${platform}`)) return 'android';
  if (/iPhone|iPad|iPod/i.test(ua) || (/Mac/i.test(platform) && navigator.maxTouchPoints > 1)) return 'ios';
  if (/Windows|Win32/i.test(`${ua} ${platform}`)) return 'windows';
  if (/Mac/i.test(`${ua} ${platform}`)) return 'macos';
  if (/Linux|CrOS/i.test(`${ua} ${platform}`)) return 'linux';
  return null;
}

const names = { linux: 'Linux', macos: 'macOS', windows: 'Windows', android: 'Android', ios: 'iPhone and iPad' };
const tabs = [...document.querySelectorAll('[role="tab"]')];

function selectPlatform(platform, focus = false) {
  for (const tab of tabs) {
    const selected = tab.dataset.platform === platform;
    tab.setAttribute('aria-selected', String(selected));
    tab.tabIndex = selected ? 0 : -1;
    document.getElementById(tab.getAttribute('aria-controls')).hidden = !selected;
    if (selected && focus) tab.focus();
  }
}

// Without JavaScript every platform stays listed, so tab semantics are added here.
for (const tab of tabs) {
  const panel = document.getElementById(tab.getAttribute('aria-controls'));
  panel.setAttribute('role', 'tabpanel');
  panel.setAttribute('aria-labelledby', tab.id);
}
const detected = detectedPlatform();
document.querySelector('.platform-tabs').hidden = false;
selectPlatform(detected ?? 'linux');
if (detected === 'android' || detected === 'ios') {
  // The mobile apps are not in the stores yet, so offer what works today.
  document.querySelector('#suggested-label').textContent = 'Try Shep in your browser';
  document.querySelector('#suggested-install').href = 'demo/';
} else if (detected) {
  document.querySelector('#suggested-label').textContent = `Get Shep for ${names[detected]}`;
}

tabs.forEach((tab, index) => {
  tab.addEventListener('click', () => selectPlatform(tab.dataset.platform));
  tab.addEventListener('keydown', event => {
    const step = { ArrowRight: 1, ArrowLeft: -1 }[event.key];
    const target = event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : step === undefined ? null : (index + step + tabs.length) % tabs.length;
    if (target === null) return;
    event.preventDefault();
    selectPlatform(tabs[target].dataset.platform, true);
  });
});

function followHash() {
  const platform = location.hash.slice(1);
  if (!(platform in names)) return;
  selectPlatform(platform);
  document.getElementById(platform).scrollIntoView();
}
window.addEventListener('hashchange', followHash);
followHash();

for (const button of document.querySelectorAll('.copy-button')) {
  button.hidden = false;
  button.addEventListener('click', async () => {
    const status = button.closest('.install-panel').querySelector('.copy-status');
    try {
      await navigator.clipboard.writeText(document.getElementById(button.dataset.copy).textContent);
      button.textContent = 'Copied';
      status.textContent = `Copied. Paste it into ${button.closest('.install-panel').dataset.shell} to install Shep.`;
      setTimeout(() => { button.textContent = 'Copy'; }, 2000);
    } catch {
      status.textContent = 'Copy is unavailable. Select the command above and copy it.';
    }
  });
}

// The embedded and full-screen demos follow the page's appearance. The frame
// starts on System, so it only reloads when another choice is saved or made.
const demoFrame = document.querySelector('#demo-frame');
function syncDemoAppearance() {
  const query = appearance.value === 'system' ? '' : `?appearance=${appearance.value}`;
  for (const link of document.querySelectorAll('.demo-open')) link.href = `demo/${query}`;
  const source = `demo/app/${query}`;
  if (demoFrame.getAttribute('src') !== source) demoFrame.src = source;
}
syncDemoAppearance();
appearance.addEventListener('change', syncDemoAppearance);

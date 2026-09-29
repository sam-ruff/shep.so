// The full-screen demo matches the requested appearance, and on phones and
// tablets opens a modal notice first. Closing it with the button or Escape
// returns focus to the demo. Dismissal lasts for this page view only.
(() => {
  const appearance = new URLSearchParams(location.search).get('appearance');
  if (appearance === 'light' || appearance === 'dark') document.documentElement.dataset.theme = appearance;

  const dialog = document.querySelector<HTMLDialogElement>('#demo-notice');
  const frame = document.querySelector<HTMLIFrameElement>('#demo-frame');
  const proceed = dialog?.querySelector<HTMLButtonElement>('.notice-button');
  if (!dialog || !frame || !proceed || !isMobilePlatform()) return;

  proceed.addEventListener('click', () => dialog.close());
  dialog.addEventListener('close', () => frame.focus());
  dialog.showModal();
})();

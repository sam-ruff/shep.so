// On phones and tablets the embedded demo starts behind a notice. The notice
// never takes focus or scrolls the page, and the demo stays inert until the
// visitor continues. Dismissal lasts for this page view only.
(() => {
  const notice = document.querySelector<HTMLElement>('#demo-notice');
  const frame = document.querySelector<HTMLIFrameElement>('#demo-frame');
  const proceed = notice?.querySelector<HTMLButtonElement>('.demo-notice-continue');
  if (!notice || !frame || !proceed || !isMobilePlatform()) return;

  frame.inert = true;
  notice.hidden = false;

  const dismiss = (): void => {
    notice.hidden = true;
    frame.inert = false;
    frame.focus();
  };

  proceed.addEventListener('click', dismiss);
  notice.addEventListener('keydown', event => {
    if (event.key !== 'Escape' || event.defaultPrevented) return;
    event.preventDefault();
    dismiss();
  });
})();

// The home page passes its appearance so the full-screen demo matches it.
const appearance = new URLSearchParams(location.search).get('appearance');
const frame = document.querySelector('#demo-frame');
if (['light', 'dark'].includes(appearance)) {
  frame.src = `app/?appearance=${appearance}`;
  document.body.style.background = appearance === 'dark' ? '#141316' : '#fcfcfd';
}
frame.focus();

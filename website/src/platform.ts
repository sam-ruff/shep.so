// Shared by the home page and the full-screen demo as a classic script, so
// detectedPlatform() is a page-wide function rather than a module export.

type Platform = 'linux' | 'macos' | 'windows' | 'android' | 'ios';

interface Navigator {
  readonly userAgentData?: { readonly platform?: string };
}

// Android comes before Linux, and desktop-mode iPads come before macOS.
// ChromeOS installs through its Linux environment.
function detectedPlatform(): Platform | null {
  const ua = navigator.userAgent;
  const platform = navigator.userAgentData?.platform ?? navigator.platform ?? '';
  if (/Android/i.test(`${ua} ${platform}`)) return 'android';
  if (/iPhone|iPad|iPod/i.test(ua) || (/Mac/i.test(platform) && navigator.maxTouchPoints > 1)) return 'ios';
  if (/Windows|Win32/i.test(`${ua} ${platform}`)) return 'windows';
  if (/Mac/i.test(`${ua} ${platform}`)) return 'macos';
  if (/Linux|CrOS/i.test(`${ua} ${platform}`)) return 'linux';
  return null;
}

// Phones and tablets, by platform rather than window size: a narrow desktop
// window still gets the desktop demo.
function isMobilePlatform(): boolean {
  const platform = detectedPlatform();
  return platform === 'android' || platform === 'ios';
}

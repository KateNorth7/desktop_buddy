// Google Analytics 4. Loads only when site-config.mjs carries a valid
// measurement ID; an empty ID keeps the page free of any analytics request.
const MEASUREMENT_ID = /^G-[A-Z0-9]{4,16}$/;

export function gtagScriptUrl(measurementId) {
  if (!MEASUREMENT_ID.test(measurementId))
    throw new Error("Invalid GA4 measurement ID");
  return `https://www.googletagmanager.com/gtag/js?id=${measurementId}`;
}

export function installAnalytics(measurementId) {
  const src = gtagScriptUrl(measurementId);
  window.dataLayer = window.dataLayer || [];
  window.gtag = function gtag() {
    window.dataLayer.push(arguments);
  };
  window.gtag("js", new Date());
  window.gtag("config", measurementId);
  const script = document.createElement("script");
  script.async = true;
  script.src = src;
  document.head.append(script);
}

// Safe before and after gtag.js loads; no-op when analytics is not installed.
export function trackEvent(name, params = {}) {
  if (typeof window.gtag === "function") window.gtag("event", name, params);
}

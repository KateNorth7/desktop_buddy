// Analytics consent. Google Analytics loads only after the visitor accepts;
// the choice is the one thing the page keeps in the browser.
export const CONSENT_KEY = "sumi-analytics-consent";
export const CONSENT_VALUES = Object.freeze(["accepted", "declined"]);

export function readConsent(storage) {
  try {
    const value = storage.getItem(CONSENT_KEY);
    return CONSENT_VALUES.includes(value) ? value : null;
  } catch {
    return null;
  }
}

export function writeConsent(storage, value) {
  if (!CONSENT_VALUES.includes(value)) throw new Error("Invalid consent value");
  try {
    storage.setItem(CONSENT_KEY, value);
  } catch {
    // Private mode or blocked storage: the banner simply returns next visit.
  }
}

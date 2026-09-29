import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { PrefsPanel } from "../components/PrefsPanel";
import { Onboarding } from "../components/Onboarding";
import { useAppBootstrap } from "../hooks/useAppBootstrap";
import { useModelEvents } from "../hooks/useModelEvents";
import { usePrefsStore } from "../stores/prefs";

/** Preferences window: load preferences, listen for model downloads, and
 *  show onboarding until the user completes it. */
export function PrefsRoot() {
  const { ready } = useAppBootstrap();
  useModelEvents();
  const onboardingComplete = usePrefsStore((s) => s.prefs.onboardingComplete);
  if (!ready) return null; // brief: stores not hydrated yet
  return onboardingComplete ? <PrefsPanel /> : <Onboarding />;
}

const rootEl = document.getElementById("root");
if (rootEl) {
  createRoot(rootEl).render(
    <StrictMode>
      <PrefsRoot />
    </StrictMode>
  );
} else if (import.meta.env.MODE !== "test") {
  throw new Error("No #root element found");
}

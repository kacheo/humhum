import { useRef, useState } from "react";
import { usePrefsStore } from "../../stores/prefs";
import { PermissionsStep } from "./PermissionsStep";
import { HotkeyStep } from "./HotkeyStep";
import { CleanupStep } from "./CleanupStep";
import { ModelDownloadStep } from "./ModelDownloadStep";
import { DemoStep } from "./DemoStep";
import "../../styles/tokens.css";
import "./index.css";

const STEPS = ["Permissions", "Shortcut", "Cleanup", "Model", "Try it"] as const;

/** First-run wizard. Owns the step index and a single `ready` flag that each
 *  step reports through onReady; Next/Finish is gated on it. On Finish (or the
 *  DemoStep's Skip) it persists onboardingComplete, and the prefs.tsx gate then
 *  swaps to PrefsPanel. */
export function Onboarding() {
  const [step, setStep] = useState(0);
  const [ready, setReady] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [prepareError, setPrepareError] = useState<string | null>(null);
  const preparationInFlight = useRef(false);
  const isLast = step === STEPS.length - 1;

  async function complete() {
    usePrefsStore.getState().setField("onboardingComplete", true);
    await usePrefsStore.getState().savePrefs();
  }
  async function next() {
    if (preparationInFlight.current) return;
    if (step === STEPS.length - 2) {
      preparationInFlight.current = true;
      setPreparing(true);
      setPrepareError(null);
      try {
        // Apply the draft shortcut and cleanup settings before asking the user
        // to try them. Saving does not complete onboarding.
        await usePrefsStore.getState().savePrefs();
        setReady(false);
        setStep(STEPS.length - 1);
      } catch {
        setPrepareError("Could not apply your settings. Try again before starting dictation.");
      } finally {
        preparationInFlight.current = false;
        setPreparing(false);
      }
      return;
    }
    if (isLast) { void complete(); return; }
    setReady(false);
    setStep((s) => s + 1);
  }
  function back() {
    if (preparationInFlight.current) return;
    setPrepareError(null);
    setReady(false);
    setStep((s) => Math.max(0, s - 1));
  }

  return (
    <div className="onboarding">
      <header className="onboarding__header"><span className="onboarding__brand">Welcome to Humhum</span><span>Step {step + 1} of {STEPS.length}</span></header>
      <ol className="onboarding__progress" aria-label="Setup progress">
        {STEPS.map((label, i) => (
          <li key={label} aria-current={i === step ? "step" : undefined}
            className={`onboarding__dot${i === step ? " onboarding__dot--active" : ""}${i < step ? " onboarding__dot--done" : ""}`}>
            {i < step && <span aria-hidden="true">✓ </span>}{label}
          </li>
        ))}
      </ol>
      <div className="onboarding__body">
        {step === 0 && <PermissionsStep onReady={setReady} />}
        {step === 1 && <HotkeyStep onReady={setReady} />}
        {step === 2 && <CleanupStep onReady={setReady} />}
        {step === 3 && <ModelDownloadStep onReady={setReady} />}
        {step === 4 && <DemoStep onReady={setReady} onSkip={() => void complete()} />}
      </div>
      {prepareError && <p className="onb-error" role="alert">{prepareError}</p>}
      <footer className="onboarding__nav">
        <button type="button" onClick={back} disabled={step === 0 || preparing}>Back</button>
        <button type="button" className="onboarding__next primary-action" onClick={() => void next()} disabled={!ready || preparing}>
          {preparing ? "Preparing…" : isLast ? "Finish" : "Next"}
        </button>
      </footer>
    </div>
  );
}

import { useState } from "react";
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
  const isLast = step === STEPS.length - 1;

  async function complete() {
    usePrefsStore.getState().setField("onboardingComplete", true);
    await usePrefsStore.getState().savePrefs();
  }
  function next() {
    if (isLast) { void complete(); return; }
    setReady(false);
    setStep((s) => s + 1);
  }
  function back() {
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
      <footer className="onboarding__nav">
        <button type="button" onClick={back} disabled={step === 0}>Back</button>
        <button type="button" className="onboarding__next primary-action" onClick={next} disabled={!ready}>
          {isLast ? "Finish" : "Next"}
        </button>
      </footer>
    </div>
  );
}

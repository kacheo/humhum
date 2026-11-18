import { useLayoutEffect, useRef, useState } from "react";
import { usePrefsStore } from "../../stores/prefs";
import { Icon } from "../Icon";
import { PermissionsStep } from "./PermissionsStep";
import { HotkeyStep } from "./HotkeyStep";
import { CleanupStep } from "./CleanupStep";
import { ModelDownloadStep } from "./ModelDownloadStep";
import { DemoStep } from "./DemoStep";
import "../../styles/tokens.css";
import "./index.css";

const STEPS = [
  { label: "Permissions", icon: "shield" },
  { label: "Shortcut", icon: "keyboard" },
  { label: "Cleanup", icon: "sparkles" },
  { label: "Model", icon: "download" },
  { label: "Try it", icon: "mic" },
] as const;

/** First-run wizard. Each step reports when the user can advance. */
export function Onboarding() {
  const [step, setStep] = useState(0);
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const saveInFlight = useRef(false);
  const bodyRef = useRef<HTMLElement>(null);
  const previousStep = useRef(step);
  const isLast = step === STEPS.length - 1;

  useLayoutEffect(() => {
    if (previousStep.current === step) return;
    previousStep.current = step;
    const body = bodyRef.current;
    if (!body) return;
    body.scrollTop = 0;
    body.querySelector<HTMLElement>("h2")?.focus({ preventScroll: true });
  }, [step]);

  async function saveSetup(finish: boolean) {
    if (busy || saveInFlight.current) return;
    saveInFlight.current = true;
    setSaving(true);
    setSaveError(null);
    const store = usePrefsStore.getState();
    try {
      if (finish) store.setField("onboardingComplete", true);
      await store.savePrefs();
      if (!finish) {
        // Apply the shortcut and cleanup settings before trying dictation.
        setReady(false);
        setStep(STEPS.length - 1);
      }
    } catch {
      if (finish) store.setField("onboardingComplete", false);
      setSaveError(finish
        ? "Could not finish setup. Try again."
        : "Could not apply your settings. Try again before starting dictation.");
    } finally {
      saveInFlight.current = false;
      setSaving(false);
    }
  }

  function next() {
    if (!ready || busy || saveInFlight.current) return;
    if (step >= STEPS.length - 2) {
      void saveSetup(isLast);
      return;
    }
    setReady(false);
    setStep((current) => current + 1);
  }

  function back() {
    if (busy || saveInFlight.current) return;
    setSaveError(null);
    setReady(false);
    setStep((current) => Math.max(0, current - 1));
  }

  return (
    <div className="onboarding">
      <header className="onboarding__sidebar">
        <div className="onboarding__brand">
          <span className="onboarding__brand-icon"><Icon name="mic" size={20} /></span>
          <h1>Set up Humhum</h1>
        </div>
        <ol className="onboarding__progress" role="list" aria-label="Setup progress">
          {STEPS.map(({ label, icon }, index) => (
            <li
              key={label}
              aria-current={index === step ? "step" : undefined}
              className="onboarding__step-marker"
            >
              <span className="onboarding__step-icon">
                <Icon name={index < step ? "check" : icon} size={16} />
              </span>
              {label}
              {index < step && <span className="onboarding__sr-only"> (completed)</span>}
            </li>
          ))}
        </ol>
      </header>
      <main className="onboarding__body" ref={bodyRef}>
        {step === 0 && <PermissionsStep onReady={setReady} />}
        {step === 1 && <HotkeyStep onReady={setReady} />}
        {step === 2 && <CleanupStep onReady={setReady} onBusyChange={setBusy} />}
        {step === 3 && <ModelDownloadStep onReady={setReady} />}
        {step === 4 && <DemoStep onReady={setReady} />}
      </main>
      <footer className="onboarding__footer">
        {saveError && <p className="onb-error" role="alert">{saveError}</p>}
        <div className="onboarding__nav">
          <button type="button" onClick={back} disabled={step === 0 || saving || busy}>
            <Icon name="arrow-left" size={16} /> Back
          </button>
          <div className="onboarding__forward">
            {isLast && (
              <button type="button" className="demo-skip" onClick={() => void saveSetup(true)} disabled={saving}>
                Skip for now
              </button>
            )}
            <button
              type="button"
              className="onboarding__next primary-action"
              onClick={next}
              disabled={!ready || saving || busy}
            >
              {saving ? (isLast ? "Finishing…" : "Preparing…") : isLast ? "Finish" : "Next"}
              <Icon name={isLast ? "check" : "arrow-right"} size={16} />
            </button>
          </div>
        </div>
      </footer>
    </div>
  );
}

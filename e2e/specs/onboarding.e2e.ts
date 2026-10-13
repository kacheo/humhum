import { switchToPrefs } from "../support/prefsWindow.ts";

// Manual-checklist flow: onboarding gate. Fresh $HOME (no seeded prefs) +
// HUMHUM_E2E_FORCE_PERMISSIONS=1 (real TCC grants aren't obtainable
// headlessly) walks the full 5-step wizard to Finish and asserts the window
// swaps from `.onboarding` to `.prefs` once onboardingComplete is persisted.
describe("onboarding gate", () => {
  it("walks Permissions -> Shortcut -> Cleanup -> Model -> Try it -> Finish", async () => {
    await switchToPrefs();

    await expect($(".onboarding")).toBeExisting();
    await expect($("h2=Permissions")).toBeExisting();

    const next = $(".onboarding__next");
    await next.waitForEnabled({ timeout: 15000 }); // forced permissions -> ready
    await next.click();

    await expect($("h2=Shortcut")).toBeExisting();
    await next.waitForEnabled({ timeout: 15000 }); // default hotkey validates on mount
    await next.click();

    await expect($("h2=Cleanup")).toBeExisting();
    await next.waitForEnabled({ timeout: 5000 }); // always ready
    await next.click();

    await expect($("h2=Model")).toBeExisting();
    await $("button=Download").click();
    await browser.waitUntil(
      async () => (await $(".model-badge").getText()) === "Ready",
      { timeout: 30000, timeoutMsg: "base_en model never reached Ready" },
    );
    await next.waitForEnabled({ timeout: 5000 });
    await next.click();

    await expect($("h2=Try it")).toBeExisting();
    await $("button.demo-skip").click();

    await browser.waitUntil(
      async () => (await $(".prefs").isExisting()) && !(await $(".onboarding").isExisting()),
      { timeout: 15000, timeoutMsg: "did not swap from onboarding to PrefsPanel after Finish" },
    );
  });
});

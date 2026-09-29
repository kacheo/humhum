import { switchToPrefs } from "../support/prefsWindow.ts";

// Manual-checklist flow: model download progress. onboardingComplete seeded
// true + WIPZI_E2E_SHOW_PREFS_ON_STARTUP=1 lands directly on PrefsPanel
// (no tray to click). Asserts the Base (English) row's badge cycles
// Not downloaded -> Downloading n% -> Ready and the Download button disables.
describe("model download", () => {
  it("downloads the base_en model and reflects live progress", async () => {
    await switchToPrefs();
    await expect($(".prefs")).toBeExisting();

    const downloadBtn = $('[aria-label="Download Base (English)"]');
    const badge = $$(".model-badge")[0];

    await expect(badge).toHaveText("Not downloaded");
    await downloadBtn.click();

    // The mock server throttles the fixture into several chunks (see
    // support/mockServer.ts's streamThrottled) specifically so this
    // intermediate state is observable rather than racing straight to Ready.
    await browser.waitUntil(
      async () => (await badge.getText()).startsWith("Downloading"),
      { timeout: 10000, timeoutMsg: "never observed an intermediate Downloading n% state" },
    );

    await browser.waitUntil(async () => (await badge.getText()) === "Ready", {
      timeout: 30000,
      timeoutMsg: "base_en model never reached Ready",
    });
    await expect(downloadBtn).toBeDisabled();
  });
});

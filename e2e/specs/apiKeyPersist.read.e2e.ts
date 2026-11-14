import { switchToPrefs } from "../support/prefsWindow.ts";

describe("Cloud API credentials after a cold restart", () => {
  it("restores saved status and removes the key", async () => {
    await switchToPrefs();
    await $('button[role="tab"][id="prefs-tab-ai"]').click();
    await expect($('[aria-label="API key"]')).toHaveValue("");
    await expect($(".api-key-input__note")).toHaveText("Key saved in Keychain");
    await $("button=Remove key").click();
    await expect($(".api-key-input__note")).toHaveText("No key saved");
    await expect($(".api-key-input .probe-status")).toHaveText("Key removed");
  });
});

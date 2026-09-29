import { switchToPrefs } from "../support/prefsWindow.ts";

// Manual-checklist flow: settings persist across restart, phase 1/2. Toggles
// "Launch at login" and saves. run-all.ts runs this and
// settingsPersist.read.e2e.ts as two separate `wdio run` processes against
// the SAME scratch $HOME — a genuine cold app restart, not
// browser.reloadSession() (see run-all.ts's SpecDef comment for why).
describe("settings persist across restart (write)", () => {
  it("toggles Launch at login and saves", async () => {
    await switchToPrefs();
    await expect($(".prefs")).toBeExisting();

    const checkbox = $('input[type="checkbox"]');
    await expect(checkbox).not.toBeChecked();
    await checkbox.click();
    await expect(checkbox).toBeChecked();

    const saveBtn = $(".prefs__save");
    await saveBtn.waitForEnabled({ timeout: 5000 });
    await saveBtn.click();
    // Disabled again only once savePrefs()'s set_prefs invoke has resolved and
    // isDirty is cleared (see stores/prefs.ts) — i.e. the write actually landed.
    await saveBtn.waitForEnabled({ timeout: 15000, reverse: true });
  });
});

import { switchToPrefs } from "../support/prefsWindow.ts";

// Manual-checklist flow: settings persist across restart, phase 2/2. Fresh
// app process, same $HOME as settingsPersist.write.e2e.ts — asserts the
// checkbox comes up checked because it was read fresh from settings.json on
// this launch, not carried over in still-live JS state.
describe("settings persist across restart (read)", () => {
  it("Launch at login is still checked after a cold restart", async () => {
    await switchToPrefs();
    await expect($(".prefs")).toBeExisting();

    const checkbox = $('input[type="checkbox"]');
    await expect(checkbox).toBeChecked();
  });
});

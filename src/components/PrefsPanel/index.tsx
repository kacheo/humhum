import { useState } from "react";
import { usePrefsStore } from "../../stores/prefs";
import { ipc } from "../../lib/ipc";
import { openExternal } from "../../lib/opener";
import { HotkeyPicker } from "../HotkeyPicker";
import { PermissionsNotice } from "./PermissionsNotice";
import { AiSection } from "./AiSection";
import { GeneralSection } from "./GeneralSection";
import { PrefsTabs, tabId, panelId, type TabId } from "./PrefsTabs";
import "../../styles/tokens.css";
import "./index.css";

/** Preferences window. Subscribes to the prefs/recording stores and
 *  composes the settings tabs. All tabs stay mounted (toggled via `hidden`,
 *  not unmounted) so switching tabs never discards in-progress local state
 *  (an unsaved API key, an in-progress hotkey recording) and every tab's
 *  mount effect fires on first render regardless of which tab is active. */
export function PrefsPanel() {
  const [saveError, setSaveError] = useState<string | null>(null);
  const prefs = usePrefsStore((s) => s.prefs);
  const isDirty = usePrefsStore((s) => s.isDirty);
  const isSaving = usePrefsStore((s) => s.isSaving);
  const setField = usePrefsStore.getState().setField;
  const [activeTab, setActiveTab] = useState<TabId>("general");

  return (
    <div className="prefs">
      <aside className="prefs__sidebar">
        <div className="prefs__brand">Humhum<small>Preferences</small></div>
        <PrefsTabs active={activeTab} onSelect={setActiveTab} />
        <div className="prefs__source">
          <span>Humhum · AGPLv3</span>
          <button type="button" onClick={() => void openExternal("https://github.com/kacheo/humhum")}>
            Source and license
          </button>
        </div>
      </aside>

      <div className="prefs__content">
        <section
          className="prefs__tabpanel"
          role="tabpanel"
          id={panelId("general")}
          aria-labelledby={tabId("general")}
          tabIndex={0}
          hidden={activeTab !== "general"}
        >
          <header className="page-heading"><h1>General</h1><p>Make Humhum feel at home on your Mac.</p></header>
          <GeneralSection />
        </section>

        <section
          className="prefs__tabpanel"
          role="tabpanel"
          id={panelId("shortcut")}
          aria-labelledby={tabId("shortcut")}
          tabIndex={0}
          hidden={activeTab !== "shortcut"}
        >
          <header className="page-heading"><h1>Shortcut</h1><p>Hold your shortcut to dictate from any app.</p></header>
          <div className="prefs__section">
            <h2 className="prefs__heading">Dictation shortcut</h2>
            <p className="prefs__hint">Click the keys below to record a new shortcut. Press Escape to cancel.</p>
            <HotkeyPicker
              value={prefs.hotkey}
              onChange={(combo) => setField("hotkey", combo)}
              onValidate={ipc.validateHotkey}
            />
          </div>
        </section>

        <section
          className="prefs__tabpanel"
          role="tabpanel"
          id={panelId("ai")}
          aria-labelledby={tabId("ai")}
          tabIndex={0}
          hidden={activeTab !== "ai"}
        >
          <header className="page-heading"><h1>AI</h1><p>Choose how your speech becomes text.</p></header>
          <AiSection />
        </section>

        <section
          className="prefs__tabpanel"
          role="tabpanel"
          id={panelId("permissions")}
          aria-labelledby={tabId("permissions")}
          tabIndex={0}
          hidden={activeTab !== "permissions"}
        >
          <header className="page-heading"><h1>Permissions</h1><p>Manage the access Humhum needs on your Mac.</p></header>
          <div className="prefs__section">
            <PermissionsNotice />
          </div>
        </section>

      </div>

      <footer className="prefs__footer">
        <p className="prefs__save-feedback" role={saveError ? "alert" : "status"}>{saveError ?? (isSaving ? "Saving changes…" : isDirty ? "Unsaved changes" : "All changes saved")}</p>

        <button
          type="button"
          className="prefs__discard"
          disabled={!isDirty || isSaving}
          onClick={() => { setSaveError(null); usePrefsStore.getState().discardChanges(); }}
        >
          Discard
        </button>
        <button
          type="button"
          className="prefs__save primary-action"
          disabled={!isDirty || isSaving}
          onClick={() => {
            setSaveError(null);
            void usePrefsStore.getState().savePrefs().catch(() => setSaveError("Could not save. Your edits are kept. Try again."));
          }}
        >
          {isSaving ? "Saving…" : "Save"}
        </button>
      </footer>
    </div>
  );
}

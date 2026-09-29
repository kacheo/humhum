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
  const prefs = usePrefsStore((s) => s.prefs);
  const isDirty = usePrefsStore((s) => s.isDirty);
  const isSaving = usePrefsStore((s) => s.isSaving);
  const setField = usePrefsStore.getState().setField;
  const [activeTab, setActiveTab] = useState<TabId>("general");

  return (
    <div className="prefs">
      <PrefsTabs active={activeTab} onSelect={setActiveTab} />

      <div className="prefs__content">
        <section
          className="prefs__tabpanel"
          role="tabpanel"
          id={panelId("general")}
          aria-labelledby={tabId("general")}
          tabIndex={0}
          hidden={activeTab !== "general"}
        >
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
          <div className="prefs__section">
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
          <div className="prefs__section">
            <PermissionsNotice />
          </div>
        </section>

      </div>

      <footer className="prefs__footer">
        <div className="prefs__source">
          <span>Wipzi · AGPLv3</span>
          <button type="button" onClick={() => void openExternal("https://github.com/kacheo/wipzi")}>
            Source and license
          </button>
        </div>
        <button
          type="button"
          className="prefs__discard"
          disabled={!isDirty || isSaving}
          onClick={() => usePrefsStore.getState().discardChanges()}
        >
          Discard
        </button>
        <button
          type="button"
          className="prefs__save"
          disabled={!isDirty || isSaving}
          onClick={() => {
            void usePrefsStore.getState().savePrefs();
          }}
        >
          {isSaving ? "Saving…" : "Save"}
        </button>
      </footer>
    </div>
  );
}

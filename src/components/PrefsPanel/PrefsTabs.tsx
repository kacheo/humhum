import { useRef } from "react";

export type TabId = "general" | "shortcut" | "ai" | "permissions";

const TABS: { id: TabId; label: string }[] = [
  { id: "general", label: "General" },
  { id: "shortcut", label: "Shortcut" },
  { id: "ai", label: "AI" },
  { id: "permissions", label: "Permissions" },
];

// Shared naming convention so PrefsPanel's tabpanels can reference the same
// ids without duplicating the string scheme.
export const tabId = (id: TabId) => `prefs-tab-${id}`;
export const panelId = (id: TabId) => `prefs-panel-${id}`;

/** WAI-ARIA 1.2 Tabs pattern (automated activation): the tablist is a single
 *  stop in the page Tab order (roving tabindex — only the active tab is
 *  tabbable); Left/Right/Home/End move focus and activate the corresponding
 *  panel immediately; Left/Right wrap at the ends. */
export function PrefsTabs({ active, onSelect }: { active: TabId; onSelect: (id: TabId) => void }) {
  const btnRefs = useRef<(HTMLButtonElement | null)[]>([]);

  function activateIndex(index: number) {
    const wrapped = (index + TABS.length) % TABS.length;
    onSelect(TABS[wrapped].id);
    btnRefs.current[wrapped]?.focus();
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLButtonElement>, index: number) {
    switch (e.key) {
      case "ArrowRight":
        e.preventDefault();
        activateIndex(index + 1);
        break;
      case "ArrowLeft":
        e.preventDefault();
        activateIndex(index - 1);
        break;
      case "Home":
        e.preventDefault();
        activateIndex(0);
        break;
      case "End":
        e.preventDefault();
        activateIndex(TABS.length - 1);
        break;
    }
  }

  return (
    <div className="prefs__tabs" role="tablist" aria-label="Preferences sections">
      {TABS.map((t, i) => {
        const isActive = active === t.id;
        return (
          <button
            key={t.id}
            ref={(el) => {
              btnRefs.current[i] = el;
            }}
            type="button"
            id={tabId(t.id)}
            role="tab"
            aria-selected={isActive}
            aria-controls={panelId(t.id)}
            tabIndex={isActive ? 0 : -1}
            className={`prefs__tab${isActive ? " prefs__tab--active" : ""}`}
            onClick={() => onSelect(t.id)}
            onKeyDown={(e) => handleKeyDown(e, i)}
          >
            {t.label}
          </button>
        );
      })}
    </div>
  );
}

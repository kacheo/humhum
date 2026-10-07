import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { PrefsTabs, type TabId } from "./PrefsTabs";

// Controlled wrapper: active is owned by the "parent" exactly as PrefsPanel does.
function ControlledTabs() {
  const [active, setActive] = useState<TabId>("general");
  return <PrefsTabs active={active} onSelect={setActive} />;
}

test("only the active tab is tabbable (roving tabindex) and aria-selected matches", () => {
  render(<ControlledTabs />);
  expect(screen.getByRole("tab", { name: "General" })).toHaveAttribute("tabIndex", "0");
  expect(screen.getByRole("tab", { name: "General" })).toHaveAttribute("aria-selected", "true");
  expect(screen.getByRole("tab", { name: "Shortcut" })).toHaveAttribute("tabIndex", "-1");
  expect(screen.getByRole("tab", { name: "Shortcut" })).toHaveAttribute("aria-selected", "false");
});

test("each tab exposes the id/aria-controls pair its panel will reference", () => {
  render(<ControlledTabs />);
  const ai = screen.getByRole("tab", { name: "AI" });
  expect(ai.id).toBe("prefs-tab-ai");
  expect(ai).toHaveAttribute("aria-controls", "prefs-panel-ai");
});

test("ArrowDown moves focus and activates the next tab, wrapping past the last", async () => {
  const user = userEvent.setup();
  render(<ControlledTabs />);
  screen.getByRole("tab", { name: "General" }).focus();

  await user.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}");
  const permissions = screen.getByRole("tab", { name: "Permissions" });
  expect(permissions).toHaveFocus();
  expect(permissions).toHaveAttribute("aria-selected", "true");

  await user.keyboard("{ArrowDown}"); // wraps
  const general = screen.getByRole("tab", { name: "General" });
  expect(general).toHaveFocus();
  expect(general).toHaveAttribute("aria-selected", "true");
});

test("ArrowUp from the first tab wraps to the last", async () => {
  const user = userEvent.setup();
  render(<ControlledTabs />);
  screen.getByRole("tab", { name: "General" }).focus();
  await user.keyboard("{ArrowUp}");
  const permissions = screen.getByRole("tab", { name: "Permissions" });
  expect(permissions).toHaveFocus();
  expect(permissions).toHaveAttribute("aria-selected", "true");
});

test("Home/End jump to the first/last tab", async () => {
  const user = userEvent.setup();
  render(<ControlledTabs />);
  screen.getByRole("tab", { name: "AI" }).focus();
  await user.keyboard("{End}");
  expect(screen.getByRole("tab", { name: "Permissions" })).toHaveFocus();
  await user.keyboard("{Home}");
  expect(screen.getByRole("tab", { name: "General" })).toHaveFocus();
});

test("clicking a tab still activates it (mouse behavior unchanged)", async () => {
  const user = userEvent.setup();
  render(<ControlledTabs />);
  await user.click(screen.getByRole("tab", { name: "Permissions" }));
  expect(screen.getByRole("tab", { name: "Permissions" })).toHaveAttribute("aria-selected", "true");
});

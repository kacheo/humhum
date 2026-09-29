import { switchToPrefs } from "../support/prefsWindow.ts";

const mockUrl = process.env.WIPZI_E2E_MOCK_URL;
if (!mockUrl) {
  throw new Error("WIPZI_E2E_MOCK_URL not set — this spec must be run via run-all.ts");
}

async function setApiKeyResult(result: "success" | "fail"): Promise<void> {
  const res = await fetch(`${mockUrl}/_control/api-key-result`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ result }),
  });
  if (!res.ok) throw new Error(`mock control call failed: ${res.status}`);
}

// Manual-checklist flow: API key test button. Cloud provider defaults to
// OpenAI (see prefs default), so the mock's /openai/models route is what
// test_api_key hits. Flips the mock's result via its control-plane route to
// exercise both the success and failure UI states.
describe("API key test button", () => {
  it("shows success then failure based on the backend response", async () => {
    await switchToPrefs();
    await expect($(".prefs")).toBeExisting();
    await $('button[role="tab"][id="prefs-tab-ai"]').click();
    await browser.execute(() => {
      const select = [...document.querySelectorAll<HTMLSelectElement>("#prefs-panel-ai select")]
        .find((element) => element.querySelector('option[value="api"]'));
      if (!select) throw new Error("cleanup mode selector missing");
      select.value = "api";
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await expect($('[aria-label="API key"]')).toBeExisting();

    const keyInput = $('[aria-label="API key"]');
    const testBtn = $(".api-key-input button");
    const result = $(".api-key-input .probe-status");

    await keyInput.setValue("sk-e2e-test-key");

    await setApiKeyResult("success");
    await testBtn.click();
    await expect(result).toHaveText("Key verified", { wait: 15000 });
    await expect(result).not.toHaveElementClass("probe-status--error");

    await setApiKeyResult("fail");
    await testBtn.click();
    await browser.waitUntil(
      async () => (await result.getAttribute("class"))?.includes("probe-status--error") ?? false,
      { timeout: 15000, timeoutMsg: "result never showed the error state after a failing key" },
    );
  });
});

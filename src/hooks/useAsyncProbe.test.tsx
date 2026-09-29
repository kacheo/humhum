import { act, renderHook } from "@testing-library/react";
import { vi } from "vitest";
import { useAsyncProbe } from "./useAsyncProbe";

test("initial state is idle with no result", () => {
  const { result } = renderHook(() => useAsyncProbe());
  expect(result.current.testing).toBe(false);
  expect(result.current.result).toBeNull();
});

test("a successful action reports ok:true with its message, and testing flips true -> false", async () => {
  const { result } = renderHook(() => useAsyncProbe());
  let resolveAction!: (msg: string) => void;
  const action = vi.fn(
    () =>
      new Promise<string>((res) => {
        resolveAction = res;
      })
  );

  act(() => {
    void result.current.run(action);
  });
  expect(result.current.testing).toBe(true);
  expect(result.current.result).toBeNull();

  await act(async () => {
    resolveAction("all good");
  });
  expect(result.current.testing).toBe(false);
  expect(result.current.result).toEqual({ ok: true, msg: "all good" });
});

test("a thrown Error is captured as ok:false with its .message", async () => {
  const { result } = renderHook(() => useAsyncProbe());
  await act(async () => {
    await result.current.run(async () => {
      throw new Error("boom");
    });
  });
  expect(result.current.testing).toBe(false);
  expect(result.current.result).toEqual({ ok: false, msg: "boom" });
});

test("a thrown non-Error value is captured via String(e)", async () => {
  const { result } = renderHook(() => useAsyncProbe());
  await act(async () => {
    await result.current.run(async () => {
      // Deliberately throwing a non-Error to hit the String(e) fallback.
      throw "raw-string-failure";
    });
  });
  expect(result.current.result).toEqual({ ok: false, msg: "raw-string-failure" });
});

test("starting a new run clears the previous result immediately, before the new one resolves", async () => {
  const { result } = renderHook(() => useAsyncProbe());
  await act(async () => {
    await result.current.run(async () => "first");
  });
  expect(result.current.result).toEqual({ ok: true, msg: "first" });

  let resolveSecond!: (msg: string) => void;
  act(() => {
    void result.current.run(
      () =>
        new Promise<string>((res) => {
          resolveSecond = res;
        })
    );
  });
  expect(result.current.result).toBeNull();
  expect(result.current.testing).toBe(true);

  await act(async () => {
    resolveSecond("second");
  });
  expect(result.current.result).toEqual({ ok: true, msg: "second" });
});

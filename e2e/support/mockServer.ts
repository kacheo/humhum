// Local stand-in for third-party HTTP endpoints the E2E flows touch:
// OpenAI/Anthropic (API key test) and Hugging Face
// (model download). Plain node:http — no new runtime dependency, one process
// per `wdio run` (i.e. per spec file, since run-all.mjs invokes wdio once per
// spec). Control-plane routes let a spec mutate mock state from a separate
// process via plain fetch(), since the spec runs in a WDIO worker process,
// not this one.
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { createReadStream, statSync } from "node:fs";

export interface MockServerState {
  apiKeyResult: "success" | "fail";
}

export interface MockServerHandle {
  url: string;
  port: number;
  state: MockServerState;
  close(): Promise<void>;
}

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    req.on("error", reject);
  });
}

function sendJson(res: ServerResponse, status: number, body: unknown): void {
  const json = JSON.stringify(body);
  res.writeHead(status, { "content-type": "application/json" });
  res.end(json);
}

// Streams `path` back in small chunks with a short delay between them, so a
// 2 MiB fixture produces several distinct `model-download-progress` ticks
// instead of arriving as one giant OS-buffered read.
function streamThrottled(res: ServerResponse, filePath: string): void {
  const { size } = statSync(filePath);
  res.writeHead(200, {
    "content-type": "application/octet-stream",
    "content-length": String(size),
  });
  const CHUNK = 64 * 1024;
  const stream = createReadStream(filePath, { highWaterMark: CHUNK });
  stream.on("data", (chunk) => {
    stream.pause();
    res.write(chunk, () => {
      setTimeout(() => stream.resume(), 15);
    });
  });
  stream.on("end", () => res.end());
  stream.on("error", () => res.end());
}

/**
 * Start the mock server on an ephemeral port. Default API key test succeeds — flip via the `/_control/*` routes or by mutating
 * `handle.state` directly (control routes are for cross-process specs;
 * direct mutation works when the caller shares this process, e.g. tests for
 * the mock server itself).
 */
export function startMockServer(modelFixturePath: string): Promise<MockServerHandle> {
  const state: MockServerState = {
    apiKeyResult: "success",
  };

  const server = createServer(async (req, res) => {
    const url = new URL(req.url ?? "/", "http://localhost");

    try {
      // --- Control plane -------------------------------------------------
      if (url.pathname === "/_control/api-key-result" && req.method === "POST") {
        const body = JSON.parse(await readBody(req));
        state.apiKeyResult = body.result === "fail" ? "fail" : "success";
        return sendJson(res, 200, { ok: true });
      }

      // --- OpenAI test-key probe (GET {base}/models) ----------------------
      if (url.pathname === "/openai/models" && req.method === "GET") {
        if (state.apiKeyResult === "success") {
          return sendJson(res, 200, { data: [] });
        }
        return sendJson(res, 401, { error: "invalid api key" });
      }

      // --- Anthropic test-key probe (POST {base}/messages) ----------------
      if (url.pathname === "/anthropic/messages" && req.method === "POST") {
        if (state.apiKeyResult === "success") {
          return sendJson(res, 200, { id: "msg_e2e", content: [] });
        }
        return sendJson(res, 401, { error: "invalid x-api-key" });
      }

      // --- Hugging Face model binary (throttled) ---------------------------
      if (url.pathname === "/models/base-en-fixture.bin" && req.method === "GET") {
        return streamThrottled(res, modelFixturePath);
      }

      res.writeHead(404);
      res.end();
    } catch (e) {
      res.writeHead(500);
      res.end(String(e));
    }
  });

  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (address === null || typeof address === "string") {
        throw new Error("mock server: expected an AddressInfo");
      }
      const port = address.port;
      resolve({
        url: `http://127.0.0.1:${port}`,
        port,
        state,
        close: () => new Promise((res2) => server.close(() => res2())),
      });
    });
  });
}

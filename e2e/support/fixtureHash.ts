// Derives the model-download fixture's sha256 + byte size at run time, so
// `e2e/fixtures/models/base-en-fixture.bin` is the single source of truth —
// nothing hand-computed to drift out of sync with the checked-in file.
import { createHash } from "node:crypto";
import { createReadStream, statSync } from "node:fs";

export interface FixtureHash {
  sha256: string;
  sizeBytes: number;
}

export function hashFixture(filePath: string): Promise<FixtureHash> {
  const sizeBytes = statSync(filePath).size;
  return new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    const stream = createReadStream(filePath);
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("error", reject);
    stream.on("end", () => resolve({ sha256: hash.digest("hex"), sizeBytes }));
  });
}

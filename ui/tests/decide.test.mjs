import assert from "node:assert/strict";
import test from "node:test";
import {
  CHECK_STARTER_PACKS,
  formatConfidencePercent,
  resultsToCsv,
  sanitizeCsvCell,
} from "../decide.js";
import { CAPABILITIES } from "../images.js";

test("Check Documents starter packs, confidence labels, and safe CSV export work as expected", () => {
  assert.deepEqual(
    CHECK_STARTER_PACKS.map((p) => p.id),
    ["bills", "contracts", "applications"],
  );
  assert.equal(formatConfidencePercent(0.924), "92%");
  assert.equal(formatConfidencePercent(1.5), "100%");
  assert.equal(formatConfidencePercent(-0.2), "0%");

  assert.equal(sanitizeCsvCell("=SUM(A1:A5)"), "'=SUM(A1:A5)");
  assert.equal(sanitizeCsvCell("+123,45"), "\"'+123,45\"");
  assert.equal(sanitizeCsvCell("line1\nline2"), "line1 line2");

  const csv = resultsToCsv(
    [{ id: "q1", prompt: "Do I owe money?" }],
    [
      {
        document_name: "invoice, march.pdf",
        answers: [{ question_id: "q1", answer: "Yes", confidence: 0.94, evidence: "Due today.", part: 2, total_parts: 3 }],
      },
      {
        document_name: "receipt.pdf",
        answers: [{ question_id: "q1", answer: "Unsure", reason: "Conflicting excerpts." }],
      },
    ],
  );
  assert.equal(csv, 'Document,Do I owe money?\n"invoice, march.pdf",Yes (94%) (part 2/3) - Due today.\nreceipt.pdf,Unsure - Conflicting excerpts.');
});

test("CSV adds source coverage when part of a file was left out", () => {
  const csv = resultsToCsv([], [
    { document_name: "complete.pdf" },
    { document_name: "partial.pdf", truncated: true },
  ]);
  assert.equal(csv, "Document,Source coverage\ncomplete.pdf,\npartial.pdf,Partial import");
});

test("Apps hub exposes Check Documents in plain language", () => {
  const card = CAPABILITIES.find((c) => c.id === "decide");
  assert.ok(card, "Check Documents card missing from CAPABILITIES");
  assert.equal(card.title, "Check Documents");
  assert.match(card.detail, /bills, contracts, letters/i);
});

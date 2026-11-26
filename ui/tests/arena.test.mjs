import assert from "node:assert/strict";
import test from "node:test";
import { arenaTabLabel, digestPreview } from "../arena.js";

test("arena tab labels cover compare, dashboard, and overview", () => {
  assert.equal(arenaTabLabel("compare"), "Compare");
  assert.equal(arenaTabLabel("dashboard"), "Dashboard");
  assert.equal(arenaTabLabel("overview"), "Overview");
  assert.equal(arenaTabLabel("other"), "Compare");
});

test("digest preview keeps short hashes and truncates long ones", () => {
  assert.equal(digestPreview(""), "—");
  assert.equal(digestPreview("abc"), "abc");
  const long = "416242447301baff5498e89920db4099f590eac95905c3d69780bc35b5f9f429";
  assert.equal(digestPreview(long), `4162424473…${long.slice(-6)}`);
  assert.match(digestPreview(long), /^4162424473…/);
});

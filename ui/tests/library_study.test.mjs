import assert from "node:assert/strict";
import test from "node:test";
import { isStudyChoiceCorrect, parseStudyDeck } from "../markdown.js";
import { filterLibraryItems, libraryBadgeLabel, libraryCategoryFor } from "../everyday.js";

test("parseStudyDeck handles numbered questions, checkboxes, and bare-letter answers", () => {
  const deck = parseStudyDeck(`# Space Quiz

**Q1:** Which planet is closest to the Sun?
- Venus
- Mercury
- Mars
A: B

2. Q: Where does photosynthesis happen?
- [ ] Mitochondria
- [x] Chloroplasts
- [ ] Nucleus
`);
  assert.equal(deck.title, "Space Quiz");
  assert.equal(deck.cards.length, 2);
  assert.equal(deck.cards[0].choices.length, 3);
  assert.ok(isStudyChoiceCorrect("Mercury", 1, deck.cards[0].answer, deck.cards[0].choices));
  assert.ok(!isStudyChoiceCorrect("Venus", 0, deck.cards[0].answer, deck.cards[0].choices));
  assert.equal(deck.cards[1].answer, "Chloroplasts");
  assert.ok(isStudyChoiceCorrect("Chloroplasts", 1, deck.cards[1].answer, deck.cards[1].choices));
  assert.ok(isStudyChoiceCorrect("Mars", 2, "(C)", ["Venus", "Mercury", "Mars"]));
  assert.ok(isStudyChoiceCorrect("Saturn", 0, "Saturn, the sixth planet", ["Saturn", "Mars"]));
  assert.ok(!isStudyChoiceCorrect("Earth", 0, "Earthquake", ["Earth", "Mars"]));
});

test("Library categorizes, labels, and filters videos, pictures, tables, and photo slides", () => {
  const items = [
    { name: "budget.csv", path: "budget.csv", kind: "csv", project_id: null },
    { name: "Launch reel", path: "videos/launch.mp4", kind: "mp4", project_id: null },
    { name: "Cabin", path: "assets/cabin.png", kind: "png", project_id: null },
    { name: "Trip slides", path: "photo-slides/trip.html", kind: "html", project_id: "p1" },
  ];
  assert.equal(libraryCategoryFor(items[0]), "tables");
  assert.equal(libraryCategoryFor(items[1]), "videos");
  assert.equal(libraryBadgeLabel(items[1]), "VIDEO");
  assert.equal(libraryBadgeLabel(items[3]), "PHOTO SLIDES");
  assert.equal(filterLibraryItems(items, { category: "videos" }).length, 1);
  assert.equal(filterLibraryItems(items, { category: "all", query: "video" }).length, 1);
  assert.equal(filterLibraryItems(items, { category: "all", projectId: "p1" }).length, 1);
});

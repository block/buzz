import assert from "node:assert/strict";
import test from "node:test";

import { foldForMatch } from "./foldForMatch.ts";

test("foldForMatch lower-cases and strips accents", () => {
  for (const [input, expected] of [
    ["Élodie", "elodie"],
    ["elodie", "elodie"],
    ["RÉUNION", "reunion"],
    ["François Ç", "francois c"],
    ["Réunion", "reunion"],
    ["Nguyễn Thị", "nguyen thi"],
    ["", ""],
  ]) {
    assert.equal(foldForMatch(input), expected, input);
  }
});

test("foldForMatch maps letters NFD leaves whole", () => {
  for (const [input, expected] of [
    ["Øresund", "oresund"],
    ["Đorđe", "dorde"],
    ["Kırmızı", "kirmizi"],
    ["ŒUVRE Cœur", "oeuvre coeur"],
    ["Æsir", "aesir"],
    ["Straße", "strasse"],
    ["GROẞ", "gross"],
    ["Ἀθῆναι", "αθηναι"],
    ["Йошкар-Ола", "иошкар-ола"],
  ]) {
    assert.equal(foldForMatch(input), expected, input);
  }
});

test("foldForMatch leaves other text alone", () => {
  for (const input of ["agent-42 🐝", "日本語", "npub1abc", "a_b.c@d"]) {
    assert.equal(foldForMatch(input), input, input);
  }
});

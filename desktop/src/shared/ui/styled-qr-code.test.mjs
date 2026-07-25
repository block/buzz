import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { StyledQrCode } from "./styled-qr-code.tsx";

const TEST_PAIRING_URI =
  "nostrpair://8f4b8db31967ce14fef970a1ff1e8eecf19a430aa1c83875e2f5be68dcac0f1a?relay=wss%3A%2F%2Frelay.example.com&secret=87d5a8cfd5807a0cb44f728b67d88d6dcb8daf99be137c158f21a50c1e913c0a&v=1";

test("renders the Wallet-style QR geometry locally", () => {
  const html = renderToStaticMarkup(
    React.createElement(StyledQrCode, {
      centerImageSrc: "/app-icon@2x.png",
      title: "Mobile pairing QR code",
      value: TEST_PAIRING_URI,
    }),
  );

  assert.match(html, /aria-label="Mobile pairing QR code"/);
  assert.match(html, /data-qr-matrix-size="57"/);
  assert.equal((html.match(/data-qr-finder-pattern=""/g) ?? []).length, 3);
  assert.ok(
    (html.match(/<circle /g) ?? []).length > 100,
    "expected the QR payload to render as individual circular data cells",
  );
  assert.match(html, /href="\/app-icon@2x\.png"/);
});

test("uses a deterministic lower-density matrix for the same payload", () => {
  const first = renderToStaticMarkup(
    React.createElement(StyledQrCode, { value: TEST_PAIRING_URI }),
  );
  const second = renderToStaticMarkup(
    React.createElement(StyledQrCode, { value: TEST_PAIRING_URI }),
  );

  assert.equal(first, second);
});

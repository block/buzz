import assert from "node:assert/strict";
import { describe, it } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  EMPTY_COMMUNITY_COMPUTE_FIXTURE,
  SMALL_COMPANY_COMMUNITY_COMPUTE_FIXTURE,
} from "../communityComputeFixtures.ts";
import { deriveCommunityComputeMapModel } from "../communityComputeMapModel.ts";
import { CommunityComputeKpiRow } from "./CommunityComputeKpiRow.tsx";
import { CommunityComputeTerritoryMap } from "./CommunityComputeTerritoryMap.tsx";

const smallModel = deriveCommunityComputeMapModel(
  SMALL_COMPANY_COMMUNITY_COMPUTE_FIXTURE.snapshot,
);

describe("community compute visual components", () => {
  it("renders contract-backed headline values", () => {
    const html = renderToStaticMarkup(
      React.createElement(CommunityComputeKpiRow, { kpis: smallModel.kpis }),
    );
    assert.match(html, /community-compute-kpi-members-sharing/);
    assert.match(html, /community-compute-kpi-contributed-vram/);
    assert.doesNotMatch(html, /community-compute-kpi-vram-allocated/);
    assert.match(html, /24 people in this community/);
    assert.match(html, />2,744 GB</);
  });

  it("renders a simple accessible territory map without dashboard modules", () => {
    const html = renderToStaticMarkup(
      React.createElement(CommunityComputeTerritoryMap, { model: smallModel }),
    );
    assert.match(html, /data-testid="community-compute-map-svg"/);
    assert.equal(
      [
        ...html.matchAll(
          /data-testid="community-compute-territory-deployment-/g,
        ),
      ].length,
      smallModel.deployments.length,
    );
    assert.match(html, /tabindex="0"/);
    assert.doesNotMatch(
      html,
      /Search models or devices|community-compute-deployment-list|community-compute-contributor-avatar|animate-mesh-inference|Map legend|Gold|selected/,
    );
  });

  it("renders a static empty state without extra controls", () => {
    const emptyModel = deriveCommunityComputeMapModel(
      EMPTY_COMMUNITY_COMPUTE_FIXTURE.snapshot,
    );
    const html = renderToStaticMarkup(
      React.createElement(CommunityComputeTerritoryMap, { model: emptyModel }),
    );
    assert.match(html, /data-testid="community-compute-map-empty"/);
    assert.doesNotMatch(html, /community-compute-deployment-list|Search/);
  });
});

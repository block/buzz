import assert from "node:assert/strict";
import { describe, it } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  EMPTY_COMMUNITY_COMPUTE_FIXTURE,
  SMALL_COMPANY_COMMUNITY_COMPUTE_FIXTURE,
} from "../communityComputeFixtures.ts";
import { deriveCommunityComputeMapModel } from "../communityComputeMapModel.ts";
import { CommunityComputeHealthScorecard } from "./CommunityComputeHealthScorecard.tsx";
import { CommunityComputeKpiRow } from "./CommunityComputeKpiRow.tsx";
import { CommunityComputeTerritoryMap } from "./CommunityComputeTerritoryMap.tsx";

const smallModel = deriveCommunityComputeMapModel(
  SMALL_COMPANY_COMMUNITY_COMPUTE_FIXTURE.snapshot,
);

describe("community compute visual components", () => {
  it("renders contract-backed KPI values", () => {
    const html = renderToStaticMarkup(
      React.createElement(CommunityComputeKpiRow, { kpis: smallModel.kpis }),
    );
    assert.match(html, /community-compute-kpi-members-sharing/);
    assert.match(html, /community-compute-kpi-contributed-vram/);
    assert.match(html, /community-compute-kpi-vram-allocated/);
    assert.match(html, /24 community members/);
    assert.match(html, />2,744 GB</);
  });

  it("renders only health states production can derive", () => {
    const html = renderToStaticMarkup(
      React.createElement(CommunityComputeHealthScorecard, {
        health: smallModel.health,
      }),
    );
    assert.match(html, /data-status="healthy"/);
    assert.match(html, /No reported issues/);
    assert.match(html, /data-testid="community-compute-health-healthy"/);
  });

  it("renders one accessible territory and list item per ready model", () => {
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
    assert.equal(
      [
        ...html.matchAll(
          /data-testid="community-compute-deployment-deployment-/g,
        ),
      ].length,
      smallModel.deployments.length,
    );
    assert.match(html, /tabindex="0"/);
    assert.match(html, /Search models or devices/);
    assert.match(html, /Model footprint/);
    assert.doesNotMatch(html, /Utilization|Region|Split across machines/);
  });

  it("keeps the production map static and supports explicit tutorial motion", () => {
    const html = renderToStaticMarkup(
      React.createElement(CommunityComputeTerritoryMap, { model: smallModel }),
    );
    assert.doesNotMatch(html, /animate-mesh-inference/);

    const tutorialHtml = renderToStaticMarkup(
      React.createElement(CommunityComputeTerritoryMap, {
        model: smallModel,
        inferenceDeploymentIds: [smallModel.deployments[0].id],
      }),
    );
    assert.match(tutorialHtml, /animate-mesh-inference/);
    assert.doesNotMatch(html, /community-compute-split-boundary/);
  });

  it("renders a static empty state without a deployment list", () => {
    const emptyModel = deriveCommunityComputeMapModel(
      EMPTY_COMMUNITY_COMPUTE_FIXTURE.snapshot,
    );
    const html = renderToStaticMarkup(
      React.createElement(CommunityComputeTerritoryMap, { model: emptyModel }),
    );
    assert.match(html, /data-testid="community-compute-map-empty"/);
    assert.doesNotMatch(html, /community-compute-deployment-list/);
  });
});
